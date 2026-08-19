//! 应用层中文分词（D0009）：写入与查询共用，保证 tsvector 语义一致。

use std::sync::LazyLock;

use jieba_rs::Jieba;

static JIEBA: LazyLock<Jieba> = LazyLock::new(Jieba::new);

/// CJK 判定（Rust stable 无 char::is_cjk，用 Unicode 区段）。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF   // CJK 统一表意
        | 0x3400..=0x4DBF // 扩展 A
        | 0xF900..=0xFAFF // 兼容表意
        | 0x3000..=0x303F // CJK 标点
        | 0x3040..=0x30FF // 假名
    )
}

/// 是否保留某 token：≥2 字的 CJK / ASCII 字母数字词。
fn keep(tok: &str) -> bool {
    if tok.trim().is_empty() {
        return false;
    }
    let has_cjk = tok.chars().any(is_cjk);
    let alnum_len = tok.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    if has_cjk {
        // 单个汉字信息量低，跳过；≥2 字保留
        tok.chars().filter(|c| is_cjk(*c)).count() >= 2
    } else {
        alnum_len >= 2 && tok.chars().all(|c| c.is_ascii_alphanumeric())
    }
}

/// 分词（写入 tsvector / 构造查询共用）。
pub fn tokenize(text: &str) -> Vec<String> {
    JIEBA
        .cut(text, true)
        .into_iter()
        .filter(|t| keep(t))
        .map(|t| t.to_lowercase())
        .collect()
}

/// 写入用：空格拼接的 token 串（配合 `to_tsvector('simple', ...)`）。
pub fn tsv_text(text: &str) -> String {
    tokenize(text).join(" ")
}

/// 查询用：`&` 连接的 tsquery 串（AND 语义）。
pub fn tsv_query(text: &str) -> String {
    tokenize(text).join(" & ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_tokenization() {
        let text = "用户偏好简洁的中文回答，住在上海用 Mac 写 Rust";
        let tokens = tokenize(text);
        assert!(tokens.iter().any(|t| t == "用户"), "tokens: {tokens:?}");
        assert!(tokens.iter().any(|t| t == "偏好"), "tokens: {tokens:?}");
        assert!(
            tokens.iter().any(|t| t.contains("上海")),
            "tokens: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| "rust" == t.as_str()),
            "tokens: {tokens:?}"
        );
        // 单汉字被过滤
        assert!(!tokens.iter().any(|t| t == "用"), "tokens: {tokens:?}");
    }

    #[test]
    fn query_and_text_forms() {
        assert!(tsv_text("你好世界").contains(' '));
        assert!(tsv_query("你好世界").contains('&'));
    }
}
