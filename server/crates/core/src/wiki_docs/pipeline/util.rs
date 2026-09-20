//! `pipeline` 的实现切片（架构治理 2026-09-20：自 pipeline.rs 纯搬移，零行为变化）。

use super::*;

/// W-4（2026-09-04）：按扩展名推断标准 MIME，优先于客户端 content_type——
/// txt 应为 text/plain、html 应为 text/html，不再被客户端传的 content_type 带偏。
pub(super) fn mime_from_name(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mime = if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".docx") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        "text/markdown"
    } else {
        return None; // 无已知扩展名 → fallback 客户端 content_type
    };
    Some(mime.to_string())
}

/// G-1（2026-09-04）：URL 归一化——去 fragment 与 utm_* tracking 参数，
/// 避免同内容因 ?utm_source=... 差异被当独立文档重复入库。解析失败时原样返回。
pub(super) fn normalize_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.set_fragment(None);
            let kept: Vec<(String, String)> = u
                .query_pairs()
                .filter(|(k, _)| !k.starts_with("utm_"))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            if kept.is_empty() {
                u.set_query(None);
            } else {
                u.query_pairs_mut().clear().extend_pairs(kept);
            }
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

pub(super) fn data_uploads() -> PathBuf {
    // EN-47：数据根解析唯一收口（wiki-engine::data_root——env 优先，fallback ~/.engram/app 且不再静默）
    engram_wiki_engine::data_root()
}

pub(super) fn extract_title_from_html(bytes: &[u8]) -> Option<String> {
    let raw = String::from_utf8_lossy(bytes);
    let html = scraper::Html::parse_document(&raw);
    let sel = scraper::Selector::parse("title").ok()?;
    html.select(&sel)
        .next()
        .map(|t| t.text().collect::<String>())
}

pub(super) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
