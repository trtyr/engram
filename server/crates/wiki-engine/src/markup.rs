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

/// W8：从正文中移除指向 `slug` 的全部 wikilink（**含 `[[slug|别名]]` 形式**）。
/// cascade 清 dead link 用——精确串替换只吃 `[[slug]]`，alias 形式会留 `|别名]]` 裸碎片。
pub fn remove_wikilinks(content: &str, slug: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < content.len() {
        if content[i..].starts_with("[[")
            && let Some(end_rel) = content[i + 2..].find("]]")
        {
            let inner = &content[i + 2..i + 2 + end_rel];
            let target = inner.split('|').next().unwrap_or("").trim();
            if target == slug {
                i += 2 + end_rel + 2; // 跳过整个链接（含别名与闭合符）
                continue;
            }
        }
        let ch = content[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// 生成阶段链接规范化：把 [[Target]] 的大小写变体对齐成真实 slug（lower_slug_map: 小写 → 真实 slug）。
/// 只修 case_mismatch（大小写差异），不补死链（无匹配原样保留）；[[slug|别名]] 只对齐 slug 保留别名。
pub fn normalize_wikilinks(
    content: &str,
    lower_slug_map: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < content.len() {
        if content[i..].starts_with("[[")
            && let Some(end_rel) = content[i + 2..].find("]]")
        {
            let inner = &content[i + 2..i + 2 + end_rel];
            match inner.split_once('|') {
                Some((slug, alias)) => {
                    let slug = slug.trim();
                    let real = lower_slug_map
                        .get(&slug.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| slug.to_string());
                    out.push_str("[[");
                    out.push_str(&real);
                    out.push('|');
                    out.push_str(alias);
                    out.push_str("]]");
                }
                None => {
                    let slug = inner.trim();
                    let real = lower_slug_map
                        .get(&slug.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| slug.to_string());
                    out.push_str("[[");
                    out.push_str(&real);
                    out.push_str("]]");
                }
            }
            i += 2 + end_rel + 2;
            continue;
        }
        let ch = content[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
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

    #[test]
    fn w8_remove_wikilinks_handles_alias() {
        // W8：别名形式整链移除，不留 `|别名]]` 裸碎片
        let md = "参见 [[dead]] 与 [[dead|别名]]，以及 [[live|活链]]。";
        let out = remove_wikilinks(md, "dead");
        assert!(!out.contains("dead"), "{out}");
        assert!(!out.contains("别名"), "别名形式的链接体必须整体移除: {out}");
        assert!(out.contains("[[live|活链]]"), "活链不受影响: {out}");

        // 中文 slug 与重复出现（移除后留双空格属正常——链接占位两侧空格保留）
        let md2 = "前 [[中文页]] 中 [[中文页|显示]] 后";
        let out2 = remove_wikilinks(md2, "中文页");
        assert_eq!(out2.trim(), "前  中  后");

        // 无关内容原样保留（含多字节字符边界）
        let md3 = "# 标题\n\n普通文字 🎉 保留。";
        assert_eq!(remove_wikilinks(md3, "ghost"), md3);
    }

    #[test]
    fn normalize_wikilinks_aligns_case_variants() {
        use std::collections::HashMap;
        let map: HashMap<String, String> = [
            ("engram".to_string(), "engram".to_string()),
            ("rust异步".to_string(), "rust异步".to_string()),
        ]
        .into();
        let md = "参见 [[Engram]] 与 [[ENGRAM|向量库]]，还有 [[不存在页]] 与 [[rust异步]]。";
        let out = normalize_wikilinks(md, &map);
        assert_eq!(
            out,
            "参见 [[engram]] 与 [[engram|向量库]]，还有 [[不存在页]] 与 [[rust异步]]。"
        );
    }
}
