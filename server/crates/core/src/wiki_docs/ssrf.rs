//! SSRF 安全抓取：仅 http/https、私网全拒、重定向逐跳复检、大小/超时限制。

use std::net::IpAddr;

/// 抓取结果。
#[derive(Debug)]
pub struct FetchedPage {
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
    pub final_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("拒绝：非 http/https 协议")]
    Scheme,
    #[error("拒绝：目标地址属于私网/保留段")]
    PrivateAddress,
    #[error("拒绝：重定向超过 3 次")]
    TooManyRedirects,
    #[error("拒绝：响应超过大小上限")]
    TooLarge,
    /// HTTP 状态非 2xx（W-1/W-2 2026-09-04：4xx 与 429/5xx 在管道侧分类处理）。
    #[error("HTTP {0}")]
    Status(u16),
    #[error("抓取失败：{0}")]
    Network(String),
    #[error("DNS 解析失败：{0}")]
    Dns(String),
}

/// IP 是否私网/保留/环回/链路本地（IPv4 + IPv6）。
/// IPv4 私有/保留网段表：`(网络地址, 前缀长度)`——判定统一查表，
/// 避免一长串 `||` 分支（架构治理 判据1c：本函数 CC≈23 → ≈8）。
/// 逐条对应原实现：0/8、10/8、100.64/10 CGNAT、127/8、169.254/16、
/// 172.16/12、192.0.2/24 TEST-NET、192.168/16、198.18/15、224/4 组播、
/// 240/4 保留、255.255.255.255 广播。
const V4_PRIVATE_PREFIXES: &[(u32, u32)] = &[
    (0x0000_0000, 8),
    (0x0A00_0000, 8),
    (0x6440_0000, 10),
    (0x7F00_0000, 8),
    (0xA9FE_0000, 16),
    (0xAC10_0000, 12),
    (0xC000_0200, 24),
    (0xC0A8_0000, 16),
    (0xC612_0000, 15),
    (0xE000_0000, 4),
    (0xF000_0000, 4),
    (0xFFFF_FFFF, 32),
];

/// IPv4 是否落在私有/保留网段（查表；掩码按前缀长度生成）。
fn is_private_v4(v4: std::net::Ipv4Addr) -> bool {
    let n = u32::from(v4);
    V4_PRIVATE_PREFIXES
        .iter()
        .any(|(net, bits)| n & (u32::MAX << (32 - bits)) == net & (u32::MAX << (32 - bits)))
}

pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xFE00) == 0xFC00 // ULA fc00::/7
                || (v6.segments()[0] & 0xFFC0) == 0xFE80 // 链路本地 fe80::/10
                || (v6.segments()[0] & 0xFF00) == 0xFF00 // 组播 ff00::/8
                || v6.to_ipv4_mapped().is_some_and(is_private_ip_v4_mapped)
        }
    }
}

fn is_private_ip_v4_mapped(v4: std::net::Ipv4Addr) -> bool {
    is_private_ip(IpAddr::V4(v4))
}

/// 解析并校验 URL 的全部 A/AAAA 记录。返回可用于 pinning 的地址列表。
/// 容器/受限网络常无 IPv6 路由而 DNS 回 AAAA——有 v4 时仅用 v4，纯 v6 域名保留。
async fn resolve_validated(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, FetchError> {
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| FetchError::Dns(e.to_string()))?
        .collect();
    if addrs.is_empty() {
        return Err(FetchError::Dns("无解析结果".into()));
    }
    let filtered: Vec<_> = if addrs.iter().any(|a| a.is_ipv4()) {
        addrs.iter().filter(|a| a.is_ipv4()).copied().collect()
    } else {
        addrs
    };
    for a in &filtered {
        if is_private_ip(a.ip()) {
            return Err(FetchError::PrivateAddress);
        }
    }
    Ok(filtered)
}

/// 安全抓取（重定向手动逐跳复检；每跳 DNS 结果 pin 住防 rebinding）。
/// 配置了 HTTP(S)_PROXY 时连接由代理发起——本机 DNS pinning 失去意义（代理
/// 自行解析目标），但 **K3：私网校验不跳过**，字面量与常规解析结果仍拒绝；
/// 残余风险是代理侧 DNS rebinding，需在部署文档声明「代理出口可信」。
pub async fn safe_fetch(
    url: &str,
    max_bytes: usize,
    timeout: std::time::Duration,
) -> Result<FetchedPage, FetchError> {
    let via_proxy = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
        .is_ok();
    safe_fetch_opts(url, max_bytes, timeout, via_proxy).await
}

/// `safe_fetch` 的可测形态：`via_proxy` 显式传入（不读环境变量）。
pub async fn safe_fetch_opts(
    url: &str,
    max_bytes: usize,
    timeout: std::time::Duration,
    via_proxy: bool,
) -> Result<FetchedPage, FetchError> {
    let mut current = url.to_string();
    for _hop in 0..4 {
        let parsed = reqwest::Url::parse(&current)
            .map_err(|e| FetchError::Network(format!("URL 非法: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(FetchError::Scheme);
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::Network("URL 无主机".into()))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(80);

        // K3：两种模式都做 DNS 解析 + 私网校验（代理模式只是不 pin）
        let addrs = resolve_validated(&host, port).await?;

        // DNS pinning：逐跳构建 client，只对已校验地址解析（防 rebinding）；
        // 代理模式跳过 pin——连接由代理发起，本机 resolve 无效
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .user_agent("engram/1.0");
        if !via_proxy {
            for a in &addrs {
                builder = builder.resolve(&host, *a);
            }
        }
        let pinned = builder
            .build()
            .map_err(|e| FetchError::Network(e.to_string()))?;

        let resp = pinned
            .get(parsed.clone())
            .send()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;

        // 重定向：换目标继续（下一轮重新校验）
        if resp.status().is_redirection() {
            if let Some(loc) = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                current = parsed
                    .join(loc)
                    .map_err(|e| FetchError::Network(e.to_string()))?
                    .to_string();
                continue;
            }
            return Err(FetchError::Network("重定向缺少 Location".into()));
        }

        if !resp.status().is_success() {
            return Err(FetchError::Status(resp.status().as_u16()));
        }

        // 大小限制（Content-Length 预检 + 流式读封顶）
        if let Some(len) = resp.content_length()
            && len as usize > max_bytes
        {
            return Err(FetchError::TooLarge);
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let mut bytes = Vec::with_capacity(8192);
        let mut resp = resp;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?
        {
            if bytes.len() + chunk.len() > max_bytes {
                return Err(FetchError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(FetchedPage {
            content_type,
            bytes,
            final_url: current,
        });
    }
    Err(FetchError::TooManyRedirects)
}
