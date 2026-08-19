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
    #[error("抓取失败：{0}")]
    Network(String),
    #[error("DNS 解析失败：{0}")]
    Dns(String),
}

/// IP 是否私网/保留/环回/链路本地（IPv4 + IPv6）。
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private() // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local() // 169.254/16
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.octets()[0] == 100 && v4.octets()[1] & 0xC0 == 64 // 100.64/10 CGNAT
                || v4.octets()[0] & 0xF0 == 224 // 组播
                || v4.octets()[0] & 0xF0 == 240 // 保留
                || v4.octets()[0] == 0 // 0/8
                || v4.octets()[0] == 127 // 127/8（IPv4 映射环回）
                || v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 2 // 192.0.2/24 TEST-NET
                || v4.octets()[0] == 198 && (v4.octets()[1] == 18 || v4.octets()[1] == 19) // 198.18/15
        }
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
async fn resolve_validated(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, FetchError> {
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| FetchError::Dns(e.to_string()))?
        .collect();
    if addrs.is_empty() {
        return Err(FetchError::Dns("无解析结果".into()));
    }
    for a in &addrs {
        if is_private_ip(a.ip()) {
            return Err(FetchError::PrivateAddress);
        }
    }
    Ok(addrs)
}

/// 安全抓取（重定向手动逐跳复检；每跳 DNS 结果 pin 住防 rebinding）。
pub async fn safe_fetch(
    url: &str,
    max_bytes: usize,
    timeout: std::time::Duration,
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

        let addrs = resolve_validated(&host, port).await?;

        // DNS pinning：逐跳构建 client，只对已校验地址解析（防 rebinding）
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .user_agent("agent-memory/1.0");
        for a in &addrs {
            builder = builder.resolve(&host, *a);
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
            return Err(FetchError::Network(format!("HTTP {}", resp.status())));
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
