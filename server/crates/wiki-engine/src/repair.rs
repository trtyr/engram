//! Repair：lint 修而不只报（wiki 收录哲学线工单③）。
//!
//! 边界三级（roadmap v6，2026-09-19 AI 深思自定）：
//! - **自动做**：slug 变体死链改写 / 死链去链接化 / ≥3 页引用的缺页概念建 stub / 孤页沿出链回挂
//! - **留痕做**：同标题重复合并（冗余丢弃或内容并入；delete_page 快照兜底 + 全库链接改指）
//! - **不做**：物理删除有内容的独立页（问用户）；语义级重复发现（lint_deep + AI 处置的领地）
//!
//! 全程确定性（不调 LLM）；页面修改一律走 put_page 语义（版本快照 + frontmatter.via="ai"）。

use serde::Serialize;

/// 单条修复动作（人话明细，供报告与汇报）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RepairAction {
    /// rewrite_variant_link | delink | create_stub | attach_orphan | merge_duplicate
    pub action: String,
    /// 主作用页 slug
    pub slug: String,
    pub detail: String,
}

/// 修复报告。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RepairReport {
    pub actions: Vec<RepairAction>,
    pub checked_pages: usize,
}

/// slug 变体归一：小写 + 去除 -/_/空格——`extensionapi` 与 `extension-api` 归一后相同。
pub fn squash(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .collect()
}

/// 拆 `[[target]]` / `[[target|alias]]` 的 inner → (target, alias)。
pub fn split_link(inner: &str) -> (String, Option<String>) {
    match inner.split_once('|') {
        Some((t, a)) => (t.trim().to_string(), Some(a.trim().to_string())),
        None => (inner.trim().to_string(), None),
    }
}

/// 把 content 里指向 `target` 的链接改写：
/// - `real = Some(r)`：改指向 r（保留 alias）——变体改写 / 合并改指
/// - `real = None`：去链接化（`[[t]]`→`t`，`[[t|a]]`→`a`）
///
/// 返回 (新内容, 改写处数)。
pub fn rewrite_links(content: &str, target: &str, real: Option<&str>) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut count = 0usize;
    let mut i = 0;
    while i < content.len() {
        if content[i..].starts_with("[[")
            && let Some(end_rel) = content[i + 2..].find("]]")
        {
            let inner = &content[i + 2..i + 2 + end_rel];
            let (t, alias) = split_link(inner);
            if t == target {
                count += 1;
                match (real, alias) {
                    (Some(r), Some(a)) => out.push_str(&format!("[[{r}|{a}]]")),
                    (Some(r), None) => out.push_str(&format!("[[{r}]]")),
                    (None, Some(a)) => out.push_str(&a),
                    (None, None) => out.push_str(&t),
                }
                i += 2 + end_rel + 2;
                continue;
            }
        }
        let ch = content[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, count)
}

/// 入队 repair 任务（批次⑦ job 化，wiki 大库化 2026-09-19）——返回 job_id；任务页可查历史。
/// 幂等安全：修复是确定性的（不调 LLM），重复跑无害。
pub async fn enqueue(
    pool: &sqlx::PgPool,
    lib: uuid::Uuid,
) -> Result<uuid::Uuid, engram_jobs::types::JobError> {
    let tpl = engram_jobs::types::JobTemplate::new("wiki_repair")
        .with_payload(serde_json::json!({ "library_id": lib }));
    let queued = engram_jobs::queue::JobQueue::new(pool.clone())
        .enqueue(tpl)
        .await?;
    Ok(queued.id)
}
