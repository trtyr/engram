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

/// 查询侧领域停用词：在几乎所有原子正文出现的通用词（记忆域内容以「用户…」开头）
/// 与问句虚词——它们对相关性零贡献，却会让 FTS 腿「处处命中」，
/// 进而使零匹配向量兜底失效（v2 测试 H-B2：「用户会开直升机」经「用户」token
/// 命中满页噪声）。只过滤查询侧，不动写入侧 tsv。
const QUERY_STOPWORDS: &[&str] = &[
    "用户",
    "我们",
    "你们",
    "他们",
    "什么",
    "怎么",
    "怎样",
    "为什么",
    "哪个",
    "哪些",
    "如何",
];

/// 查询用：token 数 ≤ `max_and_tokens` 时用 AND（精确），超过用 OR（召回兜底）。
/// 避免长查询因全 AND 命中而零召回。
pub fn tsv_query_smart(text: &str, max_and_tokens: usize) -> String {
    let tokens: Vec<String> = tokenize(text)
        .into_iter()
        .filter(|t| !QUERY_STOPWORDS.contains(&t.as_str()))
        .collect();
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

// ---------- wiki 专用变体（EN-63）----------
//
// 全平台共用 `tokenize`/`tsv_text` 服务 memory 域 atoms 索引——其语义不能动。
// wiki 页检索的痛点：① slug 型 ASCII 复合词（ai-passthrough-principle）含 -/_
// 被 keep() 整词丢弃（写入不进 tsv、查询空 token）；② 长复合 CJK 词整段成 token
// 时子串不可达。本变体把 -/_ 归一为分隔符 + 用 jieba 搜索引擎模式（切粒度更细），
// **wiki 写入与查询必须同用本变体**（wiki_pages tsv 语义）；memory 域继续走旧函数。

/// wiki 分词：-/_ 归一为空格 + cut_for_search 细粒度，其余口径同 tokenize。
pub fn tokenize_wiki(text: &str) -> Vec<String> {
    let normalized: String = text
        .chars()
        .map(|c| if matches!(c, '-' | '_') { ' ' } else { c })
        .collect();
    JIEBA
        .cut_for_search(&normalized, true)
        .into_iter()
        .filter(|t| keep(t))
        .map(|t| t.to_lowercase())
        .collect()
}

/// wiki 写入用：空格拼接 token 串（配合 to_tsvector('simple', ...)）。
pub fn tsv_text_wiki(text: &str) -> String {
    tokenize_wiki(text).join(" ")
}

/// wiki 查询用：与 tsv_query_smart 同构（≤max AND 精确，超限 OR 兜底），走 wiki 分词。
pub fn tsv_query_smart_wiki(text: &str, max_and_tokens: usize) -> String {
    let tokens: Vec<String> = tokenize_wiki(text)
        .into_iter()
        .filter(|t| !QUERY_STOPWORDS.contains(&t.as_str()))
        .collect();
    if tokens.len() > max_and_tokens {
        tokens.join(" | ")
    } else {
        tokens.join(" & ")
    }
}

#[cfg(test)]
mod wiki_variant_tests {
    use super::*;

    #[test]
    fn slug_compound_splits_into_hittable_tokens() {
        // EN-63 核心：slug 复合词不再整词丢弃——三个可命中 token
        let toks = tokenize_wiki("ai-passthrough-principle");
        assert_eq!(toks, vec!["ai", "passthrough", "principle"]);
        // snake_case 同样归一
        assert!(tokenize_wiki("host_direct_run").contains(&"host".to_string()));
        // 查询侧生成可命中的 tsquery 片段
        let q = tsv_query_smart_wiki("ai-passthrough-principle", 3);
        assert_eq!(q, "ai & passthrough & principle");
    }

    #[test]
    fn cjk_subword_reachable_via_search_mode() {
        // cut_for_search：长复合词与子词都保留——「透传」可从「透传原则」达
        let toks = tokenize_wiki("机器产出原样透传原则");
        assert!(
            toks.contains(&"透传".to_string()),
            "透传 应为独立 token：{toks:?}"
        );
        let q = tsv_query_smart_wiki("透传", 3);
        assert_eq!(q, "透传");
    }

    #[test]
    fn legacy_tokenize_unchanged_for_memory_domain() {
        // memory 域零变化基线（jieba 实证）：旧 tokenize 对 slug 切出三 token
        //（单独的 "-" 被丢弃）——行为不因本单改动而变，wiki 变体才是归一入口
        assert_eq!(
            tokenize("ai-passthrough-principle"),
            vec!["ai", "passthrough", "principle"]
        );
        // 单独「透传原则」jieba 切两词（词典无此复合词）——EN-59 当时的「透传」未命中
        // 即源于此：tsv 只有词粒度 token，子串匹配靠 cut_for_search 变体补齐
        assert_eq!(tokenize("透传原则"), vec!["透传", "原则"]);
    }
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
