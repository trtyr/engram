//! 文档解析：pdf / docx / html / md / txt → 纯文本。
//!
//! 底层独立 crate（不依赖任何内部 crate）：`core` 知识域与 `wiki-engine`
//! 摄取均经此解析，避免 wiki-engine 反向依赖 core（原 Q9 环）。CPU 密集调用由调用方
//! 决定是否 spawn_blocking。

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("不支持的格式: {0}")]
    Unsupported(String),
    #[error("解析失败: {0}")]
    Failed(String),
}

/// 按文件名/Content-Type 推断格式。
pub fn detect_format(name: &str, content_type: Option<&str>) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".pdf") {
        return "pdf";
    }
    if lower.ends_with(".docx") {
        return "docx";
    }
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        return "html";
    }
    if let Some(ct) = content_type {
        if ct.contains("pdf") {
            return "pdf";
        }
        if ct.contains("html") {
            return "html";
        }
        if ct.contains("markdown") || ct.contains("text/plain") {
            return "md";
        }
    }
    "md" // 默认按文本/markdown 处理
}

/// 解析为纯文本。name 用于格式推断。
pub fn parse_bytes(
    name: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<String, ParseError> {
    match detect_format(name, content_type) {
        "pdf" => parse_pdf(bytes),
        "docx" => parse_docx(bytes),
        "html" => parse_html(bytes),
        _ => {
            let text = String::from_utf8_lossy(bytes);
            if text.trim().is_empty() {
                Err(ParseError::Failed("文件内容为空".into()))
            } else {
                Ok(text.into_owned())
            }
        }
    }
}

fn parse_pdf(bytes: &[u8]) -> Result<String, ParseError> {
    pdf_extract::extract_text_from_mem(bytes)
        .map(|s| s.replace('\u{0}', ""))
        .map_err(|e| ParseError::Failed(format!("PDF: {e}")))
}

fn parse_docx(bytes: &[u8]) -> Result<String, ParseError> {
    let docx = docx_rs::read_docx(bytes).map_err(|e| ParseError::Failed(format!("DOCX: {e}")))?;
    let mut out = String::new();

    // 段落 → 文本：遍历 children，Run 的 Text 子节点拼接
    let para_text = |p: &docx_rs::Paragraph| -> String {
        let mut line = String::new();
        for pc in &p.children {
            if let docx_rs::ParagraphChild::Run(run) = pc {
                for rc in &run.children {
                    if let docx_rs::RunChild::Text(t) = rc {
                        line.push_str(&t.text);
                    }
                }
            }
        }
        line
    };
    for child in &docx.document.children {
        match child {
            docx_rs::DocumentChild::Paragraph(p) => {
                let line = para_text(p);
                if !line.trim().is_empty() {
                    out.push_str(line.trim());
                    out.push('\n');
                }
            }
            docx_rs::DocumentChild::Table(t) => {
                for tc in &t.rows {
                    let docx_rs::TableChild::TableRow(row) = tc;
                    {
                        let cells: Vec<String> = row
                            .cells
                            .iter()
                            .map(|cc| match cc {
                                docx_rs::TableRowChild::TableCell(c) => {
                                    let texts: Vec<String> = c
                                        .children
                                        .iter()
                                        .filter_map(|content| match content {
                                            docx_rs::TableCellContent::Paragraph(p) => {
                                                Some(para_text(p))
                                            }
                                            _ => None,
                                        })
                                        .collect();
                                    texts.join(" ")
                                }
                            })
                            .collect();
                        out.push_str(&cells.join(" | "));
                        out.push('\n');
                    }
                }
            }
            _ => {}
        }
    }
    if out.trim().is_empty() {
        Err(ParseError::Failed("DOCX 正文为空".into()))
    } else {
        Ok(out)
    }
}

fn parse_html(bytes: &[u8]) -> Result<String, ParseError> {
    let raw = String::from_utf8_lossy(bytes);
    let html = scraper::Html::parse_document(&raw);
    let sel =
        scraper::Selector::parse("body").map_err(|e| ParseError::Failed(format!("sel: {e}")))?;
    let Some(body) = html.select(&sel).next() else {
        return Err(ParseError::Failed("HTML 无 body".into()));
    };
    // 只取非 script/style 元素的**直接**文本子节点（script 内 JS 是其直接文本，天然排除）
    let mut lines: Vec<String> = Vec::new();
    for node in body.descendants() {
        let Some(el) = node.value().as_element() else {
            continue;
        };
        if matches!(
            el.name(),
            "script" | "style" | "noscript" | "template" | "head"
        ) {
            continue;
        }
        for child in node.children() {
            if let Some(t) = child.value().as_text() {
                let t = t.trim();
                if !t.is_empty() {
                    lines.push(t.to_string());
                }
            }
        }
    }
    let text = lines.join("\n");
    if text.trim().is_empty() {
        Err(ParseError::Failed("HTML 正文为空".into()))
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md_passthrough() {
        let out = parse_bytes("a.md", None, "# hi\n\n内容".as_bytes()).unwrap();
        assert!(out.contains("内容"));
    }

    #[test]
    fn html_extraction() {
        let html = "<html><body><h1>标题</h1><p>正文内容</p><script>bad()</script></body></html>"
            .as_bytes();
        let out = parse_bytes("x.html", None, html).unwrap();
        assert!(out.contains("标题"));
        assert!(out.contains("正文内容"));
        assert!(!out.contains("bad()"), "script 不应出现");
    }

    #[test]
    fn corrupt_pdf_rejected_not_panicked() {
        let err = parse_bytes("x.pdf", None, b"not a pdf at all");
        assert!(err.is_err(), "损坏 PDF 应返回错误");
    }

    #[test]
    fn empty_file_rejected() {
        assert!(parse_bytes("a.md", None, b"   ").is_err());
    }
}
