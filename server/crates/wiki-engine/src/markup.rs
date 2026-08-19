//! 页面标记：[[wikilink]] 解析、frontmatter 处理、slug 规则。

/// 提取页面中的全部 [[wikilink]] 目标 slug（去重保序）。
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'['
            && bytes[i + 1] == b'['
            && let Some(end_rel) = content[i + 2..].find("]]")
        {
            let inner = &content[i + 2..i + 2 + end_rel];
            // [[slug|显示名]] → 取 slug
            let slug = inner.split('|').next().unwrap_or("").trim();
            if is_valid_slug(slug) && !out.iter().any(|s| s == slug) {
                out.push(slug.to_string());
            }
            i += 2 + end_rel + 2;
            continue;
        }
        i += 1;
    }
    out
}

/// slug 规则：中英数与连字符，1..=80 字符，禁止路径分隔符与空白。
pub fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 80
        && !s.contains('/')
        && !s.contains('\\')
        && !s.chars().any(|c| c.is_whitespace())
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '·')
}

/// frontmatter 中的 sources 数组（简单 YAML 子集解析：`sources: [a, b]` 或多行 `- a`）。
pub fn parse_frontmatter_sources(fm_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_sources = false;
    for line in fm_text.lines() {
        let t = line.trim();
        if let Some(after) = t.strip_prefix("sources:") {
            in_sources = true;
            let rest = after.trim();
            if let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                for item in inner.split(',') {
                    let item = item.trim().trim_matches('"').trim_matches('\'');
                    if !item.is_empty() {
                        out.push(item.to_string());
                    }
                }
            }
        } else if in_sources && t.starts_with("- ") {
            let item = t[2..].trim().trim_matches('"').trim_matches('\'');
            if !item.is_empty() {
                out.push(item.to_string());
            }
        } else if in_sources && !t.starts_with('-') && !t.is_empty() {
            in_sources = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikilinks_extraction() {
        let md = "参见 [[Rust异步]] 与 [[pgvector|向量库]]，还有 [[Rust异步]] 重复。[[坏 slug/路径]] 不算。";
        let links = extract_wikilinks(md);
        assert_eq!(links, vec!["Rust异步".to_string(), "pgvector".to_string()]);
    }

    #[test]
    fn slug_rules() {
        assert!(is_valid_slug("用户画像"));
        assert!(is_valid_slug("rust-async"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("a/b"));
        assert!(!is_valid_slug("has space"));
        assert!(!is_valid_slug(&"x".repeat(81)));
    }

    #[test]
    fn frontmatter_sources() {
        let inline = "sources: [\"s1\", \"s2\"]";
        assert_eq!(parse_frontmatter_sources(inline), vec!["s1", "s2"]);
        let multi = "title: x\nsources:\n  - s1\n  - s2\nother: y";
        assert_eq!(parse_frontmatter_sources(multi), vec!["s1", "s2"]);
    }
}
