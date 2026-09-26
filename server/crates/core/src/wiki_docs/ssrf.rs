//! SSRF 安全抓取：仅 http/https、私网全拒、重定向逐跳复检、大小/超时限制。
//! EN-1：私网判定的依据是**真实 DNS**（公共解析器直答，拿目标真 IP）——
//! 本机解析产物（Clash fake-ip 198.18/15 等）永不进入判断；无豁免清单机制：
//! 任何代理模式下判定永远是「真 IP 是否私网」，新增环境不需要改这里。
//! 出网连接尊重代理环境（reqwest 默认读 HTTP(S)_PROXY）；直连模式继续用本机
//! 解析做 DNS pinning（保留系统路由语义），代理模式由代理自行解析。

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

/// 本机解析，仅用于直连模式的 DNS pinning（保留系统路由语义——Clash 拦截/分流照常生效）。
/// 不做私网检查：判定已上移 `validate_real`（真 DNS）；fake-ip 段是本机解析的预期产物。
async fn resolve_pinnable(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, FetchError> {
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
    Ok(filtered)
}

// ---------- EN-1：真实 DNS 判定（判定依据与本机解析解耦，无豁免清单） ----------

/// 判定专用公共解析器（UDP 53 直答）。
const REAL_RESOLVERS: [IpAddr; 3] = [
    IpAddr::V4(std::net::Ipv4Addr::new(223, 5, 5, 5)),
    IpAddr::V4(std::net::Ipv4Addr::new(119, 29, 29, 29)),
    IpAddr::V4(std::net::Ipv4Addr::new(8, 8, 8, 8)),
];

/// 构造 DNS 查询报文（RFC 1035：RD=1，单问题，qtype A(1)/AAAA(28)，class IN）。
fn build_dns_query(id: u16, qname: &str, qtype: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(17 + qname.len());
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0x01, 0x00]); // flags: RD=1
    p.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // qdcount=1
    for label in qname.split('.').filter(|l| !l.is_empty()) {
        p.push(label.len() as u8);
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0); // 根终止
    p.extend_from_slice(&qtype.to_be_bytes());
    p.extend_from_slice(&[0, 1]);
    p
}

/// 跳过（可能压缩的）域名字段，返回消费字节数。指针 0xC0 两字节终结。
fn skip_name(p: &[u8], mut i: usize) -> usize {
    let start = i;
    loop {
        if i >= p.len() {
            return p.len() - start;
        }
        let len = p[i];
        if len & 0xC0 == 0xC0 {
            return i - start + 2;
        }
        if len == 0 {
            return i - start + 1;
        }
        i += 1 + len as usize;
    }
}

/// 解析 DNS 应答中的 A/AAAA 记录（CNAME 链后置地址照收；名字压缩指针支持）。
fn parse_dns_answers(packet: &[u8]) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    if packet.len() < 12 {
        return ips;
    }
    let qd = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let an = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut i = 12;
    for _ in 0..qd {
        i += skip_name(packet, i);
        i += 4;
    }
    for _ in 0..an {
        if i >= packet.len() {
            break;
        }
        i += skip_name(packet, i);
        if i + 10 > packet.len() {
            break;
        }
        let rtype = u16::from_be_bytes([packet[i], packet[i + 1]]);
        let rdlen = u16::from_be_bytes([packet[i + 8], packet[i + 9]]) as usize;
        let rd = i + 10;
        match rtype {
            1 if rdlen == 4 => ips.push(IpAddr::V4(std::net::Ipv4Addr::new(
                packet[rd],
                packet[rd + 1],
                packet[rd + 2],
                packet[rd + 3],
            ))),
            28 if rdlen == 16 => {
                let mut o = [0u8; 16];
                o.copy_from_slice(&packet[rd..rd + 16]);
                ips.push(IpAddr::V6(o.into()));
            }
            _ => {}
        }
        i = rd + rdlen;
    }
    ips
}

/// 真实解析：向公共解析器直答查询 A/AAAA（UDP，2s/问，先到先用，fail-closed）。
async fn real_resolve_ips(host: &str) -> Result<Vec<IpAddr>, FetchError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]); // 字面量 IP：判定即本尊
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u16 ^ (host.len() as u16))
        .unwrap_or(0x4a1c);
    for resolver in REAL_RESOLVERS {
        for qtype in [1u16, 28u16] {
            let q = build_dns_query(id, host, qtype);
            let sock = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                Ok(s) => s,
                Err(_) => continue,
            };
            if sock.connect((resolver, 53)).await.is_err() {
                continue;
            }
            if sock.send(&q).await.is_err() {
                continue;
            }
            let mut buf = vec![0u8; 1024];
            let n =
                match tokio::time::timeout(std::time::Duration::from_secs(2), sock.recv(&mut buf))
                    .await
                {
                    Ok(Ok(n)) if n >= 12 => n,
                    _ => continue,
                };
            if buf[0] != (id >> 8) as u8 || buf[1] != (id & 0xff) as u8 {
                continue; // 答非所问（id 不符）
            }
            let mut ips = parse_dns_answers(&buf[..n]);
            ips.retain(|ip| !ip.is_unspecified());
            if ips.is_empty() {
                continue;
            }
            if qtype == 1 && ips.iter().any(IpAddr::is_ipv4) {
                ips.retain(IpAddr::is_ipv4); // 有 v4 丢 v6（容器常无 v6 路由，同原口径）
            }
            return Ok(ips);
        }
    }
    Err(FetchError::Dns(
        "公共解析器均无应答——目标真实性不可判定，拒绝（fail-closed）".into(),
    ))
}

/// EN-1：私网判定改为真实 DNS——校验目标真 IP；本机解析产物不进判断，无豁免清单。
async fn validate_real(host: &str) -> Result<(), FetchError> {
    for ip in real_resolve_ips(host).await? {
        if is_private_ip(ip) {
            return Err(FetchError::PrivateAddress);
        }
    }
    Ok(())
}

/// 安全抓取（重定向手动逐跳复检）。
/// 私网判定：真实 DNS（`validate_real`，公共解析器直答拿真 IP）——本机解析产物
/// （Clash fake-ip 等）不进判断，无豁免清单。配置 HTTP(S)_PROXY 时连接由代理发起，
/// 本机 DNS pinning 跳过（代理自行解析目标）；直连模式每跳 pin 本机解析地址防 rebinding。
/// 残余风险是代理侧出口可信性，部署文档声明「代理出口可信」。
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

        // EN-1：判定=真实 DNS（validate_real）；本机解析只作直连 pinning，不进判断
        validate_real(&host).await?;
        let addrs = if via_proxy {
            Vec::new()
        } else {
            resolve_pinnable(&host, port).await?
        };

        // DNS pinning：直连模式逐跳构建 client，只对已解析地址连接（防 rebinding）；
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
        return read_page_body(resp, max_bytes, current).await;
    }
    Err(FetchError::TooManyRedirects)
}

/// 读响应体：Content-Length 预检 + 流式读封顶（TooLarge），产出 FetchedPage。
async fn read_page_body(
    resp: reqwest::Response,
    max_bytes: usize,
    current: String,
) -> Result<FetchedPage, FetchError> {
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
    Ok(FetchedPage {
        content_type,
        bytes,
        final_url: current,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_query_packet_shape() {
        let q = build_dns_query(0x1234, "zh.wikisource.org", 1);
        assert_eq!(&q[0..2], &[0x12, 0x34]); // id
        assert_eq!(&q[2..4], &[0x01, 0x00]); // RD=1
        assert_eq!(&q[4..6], &[0, 1]); // qdcount=1
        assert_eq!(&q[12], &2); // 标签 "zh" 长度
        assert_eq!(&q[13..15], b"zh");
        assert_eq!(&q[q.len() - 4..], &[0, 1, 0, 1]); // qtype=A qclass=IN
        assert_eq!(*q.last().unwrap(), 1);
    }

    #[test]
    fn parse_answers_a_with_name_pointer() {
        // 最小应答：头部(12) + 问题区 + 一条 A 记录（名字用 0xC00C 指针指回问题区）
        let mut p = vec![0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0]; // ancount=1
        p.extend_from_slice(b"\x03www\x07example\x03com\x00");
        p.extend_from_slice(&[0, 1, 0, 1]); // qtype qclass
        p.extend_from_slice(&[0xC0, 0x0C]); // 名字指针
        p.extend_from_slice(&[0, 1, 0, 1]); // A IN
        p.extend_from_slice(&[0, 0, 0, 60]); // ttl
        p.extend_from_slice(&[0, 4, 93, 184, 216, 34]);
        let ips = parse_dns_answers(&p);
        assert_eq!(
            ips,
            vec![IpAddr::V4(std::net::Ipv4Addr::new(93, 184, 216, 34))]
        );
    }

    #[test]
    fn parse_answers_aaaa_and_skips_cname() {
        // CNAME 链后跟 AAAA：CNAME 被跳过，AAAA 收下
        let mut p = vec![0, 1, 0, 0, 0, 1, 0, 2, 0, 0, 0, 0]; // ancount=2
        p.extend_from_slice(b"\x03www\x07example\x03com\x00");
        p.extend_from_slice(&[0, 28, 0, 1]);
        // 答案 1：CNAME → alias.example.com（rdlen=19）
        p.extend_from_slice(&[0xC0, 0x0C]);
        p.extend_from_slice(&[0, 5, 0, 1]); // CNAME IN
        p.extend_from_slice(&[0, 0, 0, 30]); // ttl
        p.extend_from_slice(&[0, 19]);
        p.extend_from_slice(b"\x05alias\x07example\x03com\x00");
        // 答案 2：AAAA（内联根名）
        p.extend_from_slice(&[0x00]);
        p.extend_from_slice(&[0, 28, 0, 1]); // AAAA IN
        p.extend_from_slice(&[0, 0, 0, 30]); // ttl
        p.extend_from_slice(&[0, 16]);
        p.extend_from_slice(&[0x26, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0a, 0x01]);
        let ips = parse_dns_answers(&p);
        assert!(ips.iter().all(|ip| ip.is_ipv6()));
        assert_eq!(ips.len(), 1);
    }

    #[test]
    fn private_literal_targets_rejected_without_network() {
        // 字面量 IP 判定即本尊——真实内网/元数据地址在 validate_real 即拒（不触网）
        for h in ["127.0.0.1", "169.254.169.254", "10.1.2.3", "192.168.1.1"] {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let r = rt.block_on(validate_real(h));
            assert!(matches!(r, Err(FetchError::PrivateAddress)), "{h}");
        }
    }

    #[test]
    fn public_literal_targets_pass() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for h in ["93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946"] {
            let r = rt.block_on(validate_real(h));
            assert!(r.is_ok(), "{h}");
        }
    }
}
