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

/// 查询用：`&` 连接的 tsquery 串（AND 语义，精确匹配）。
pub fn tsv_query(text: &str) -> String {
    tokenize(text).join(" & ")
}

/// 查询用：`|` 连接的 tsquery 串（OR 语义，召回优先，长查询兜底）。
pub fn tsv_query_or(text: &str) -> String {
    tokenize(text).join(" | ")
}

/// 查询用：token 数 ≤ `max_and_tokens` 时用 AND（精确），超过用 OR（召回兜底）。
/// 避免长查询因全 AND 命中而零召回。
pub fn tsv_query_smart(text: &str, max_and_tokens: usize) -> String {
    let tokens = tokenize(text);
    if tokens.len() > max_and_tokens {
        tokens.join(" | ")
    } else {
        tokens.join(" & ")
    }
}

/// K7：查询 token 判空。单字 / 纯标点 / 单字母经 `keep()` 过滤后为空——
/// 空串送进 `to_tsquery` 只会产生空 tsquery 的 NOTICE 并静默返回零命中。
/// FTS 调用方应先用本函数判空短路（无查询向量时直接返回空结果），
/// **不要用哨兵串**（如 `'!'`——实测报 `no operand in tsquery` 真 500）。
pub fn has_query_tokens(text: &str) -> bool {
    !tokenize(text).is_empty()
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

    #[test]
    fn or_and_smart_query_forms() {
        // OR 语义：`|` 连接
        assert!(tsv_query_or("你好世界").contains('|'));
        // 短查询 → AND（精确）
        assert!(tsv_query_smart("你好世界", 3).contains('&'));
        // 长查询 → OR（召回兜底，避免全 AND 零召回）
        let long = tsv_query_smart("用户偏好简洁的中文回答", 3);
        assert!(long.contains('|'), "长查询应走 OR: {long}");
        assert!(!long.contains('&'), "长查询不应走 AND: {long}");
    }

    #[test]
    fn has_query_tokens_filters_noise() {
        // K7：单字 / 纯标点 / 单字母 → 无有效 token
        assert!(!has_query_tokens("书"));
        assert!(!has_query_tokens("的"));
        assert!(!has_query_tokens("?? !! ,,"));
        assert!(!has_query_tokens("a"));
        assert!(!has_query_tokens("  "));
        // 正常查询 → 有 token
        assert!(has_query_tokens("Rust 记忆"));
        assert!(has_query_tokens("上海"));
    }
}
