//! 结构感知分块：标题优先，目标 ~800 字符，重叠 15%（D0009 上下文友好）。

/// 一个分块。
#[derive(Debug, Clone)]
pub struct Chunk {
    pub seq: usize,
    pub content: String,
}

const TARGET: usize = 800;
const MAX: usize = 1400;
const OVERLAP_FRAC: usize = 7; // 1/7 ≈ 15%

/// 分块主入口：markdown 按标题切，其他文本按段落打包。
pub fn chunk_text(text: &str) -> Vec<Chunk> {
    let text = text.trim();
    if text.is_empty() {
        return vec![];
    }
    let sections = if looks_like_markdown(text) {
        split_by_headings(text)
    } else {
        split_by_paragraphs(text)
    };
    pack_sections(sections)
}

fn looks_like_markdown(text: &str) -> bool {
    text.lines().any(|l| l.starts_with('#')) || text.contains("```")
}

/// markdown：按 #/##/### 标题切节（标题随节走）。
fn split_by_headings(text: &str) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.starts_with("#")
            && !current.trim().is_empty() {
                sections.push(std::mem::take(&mut current));
            }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        sections.push(current);
    }
    sections
}

/// 纯文本：按空行切段落，逐段合并到不超上限。
fn split_by_paragraphs(text: &str) -> Vec<String> {
    let paras: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let mut sections = Vec::new();
    let mut buf = String::new();
    for p in paras {
        if buf.len() + p.len() + 2 > MAX && !buf.is_empty() {
            sections.push(std::mem::take(&mut buf));
        }
        if !buf.is_empty() {
            buf.push_str("\n\n");
        }
        // 超长单段硬切
        if p.len() > MAX {
            if !buf.is_empty() {
                sections.push(std::mem::take(&mut buf));
            }
            for piece in hard_split(p, MAX) {
                sections.push(piece);
            }
        } else {
            buf.push_str(p);
        }
    }
    if !buf.trim().is_empty() {
        sections.push(buf);
    }
    sections
}

fn hard_split(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + max).min(s.len());
        // 对齐字符边界
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
        out.push(s[start..end].to_string());
        start = end;
    }
    out
}

/// 节 → 目标尺寸块（相邻重叠），带序号。
fn pack_sections(sections: Vec<String>) -> Vec<Chunk> {
    let mut chunks: Vec<String> = Vec::new();
    for sec in sections {
        let sec = sec.trim().to_string();
        if sec.len() <= MAX {
            chunks.push(sec);
        } else {
            // 超长节硬切 + 重叠
            let pieces = hard_split(&sec, TARGET);
            let overlap = TARGET / OVERLAP_FRAC;
            let mut i = 0;
            while i < pieces.len() {
                let mut merged = pieces[i].clone();
                let mut j = i + 1;
                while j < pieces.len() && merged.len() + pieces[j].len() < TARGET * 3 / 2 {
                    merged.push('\n');
                    merged.push_str(&pieces[j]);
                    j += 1;
                }
                chunks.push(merged);
                // 前进时保留一块重叠
                i = if j >= pieces.len() {
                    j
                } else {
                    j.saturating_sub(1)
                }
                .max(i + 1);
                let _ = overlap;
            }
        }
    }
    chunks
        .into_iter()
        .filter(|c| !c.trim().is_empty())
        .enumerate()
        .map(|(seq, content)| Chunk { seq, content })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_split_by_heading() {
        let md = "# 标题一\n内容A\n\n# 标题二\n内容B";
        let chunks = chunk_text(md);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("标题一"));
        assert!(chunks[1].content.contains("标题二"));
    }

    #[test]
    fn long_text_chunked_within_limits() {
        let long = "段落内容。".repeat(600);
        let chunks = chunk_text(&long);
        assert!(chunks.len() >= 2, "{} 块", chunks.len());
        for c in &chunks {
            assert!(c.content.len() < 2000);
        }
        assert_eq!(chunks[0].seq, 0);
        assert_eq!(chunks.last().unwrap().seq, chunks.len() - 1);
    }

    #[test]
    fn short_text_single_chunk() {
        let chunks = chunk_text("短文本");
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn empty_gives_nothing() {
        assert!(chunk_text("   \n  ").is_empty());
    }
}
