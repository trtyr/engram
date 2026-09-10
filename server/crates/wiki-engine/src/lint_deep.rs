//! 语义 lint（lint_deep）：LLM 深度检查页面间的矛盾声明、过时声明、重要概念缺页。
//!
//! 与结构 lint（lint.rs，纯 SQL 查死链/孤儿）互补——语义维度只有 LLM 能做。
//! 结果写入 wiki_review_items（kind=flag，via=semantic_lint），走现有人审队列，
//! 不自动改写任何页面。
//!
//! 模式出处：karpathy LLM Wiki（gist 442a6bf555914893e9891c11519de94f）的 lint 操作——
//! 「矛盾已被标记、综合已反映你所读的一切」是 wiki 复利的前提。

use engram_jobs::types::JobError;
use engram_llm::types::Purpose;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use engram_distill::llm_port::chat_json_retrying;

/// 每批送检页面数（控制单次 LLM 调用的输入规模；页面 content 另截 1600 字）。
const BATCH: usize = 6;
/// 单页送检正文截断（字符）——lint 只需要声明级语义，不需要全文。
const PAGE_SNIPPET: usize = 1600;

/// 语义 lint 的一条发现。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticIssue {
    /// contradiction | stale | missing_concept
    #[serde(rename = "type")]
    pub r#type: String,
    /// 涉及的页面 slug（missing_concept 时可为空）
    #[serde(default)]
    pub pages: Vec<String>,
    /// 具体问题描述（引用双方原句更佳）
    pub detail: String,
    #[serde(default)]
    pub suggestion: String,
}

impl SemanticIssue {
    pub fn title(&self) -> String {
        let t = match self.r#type.as_str() {
            "contradiction" => "矛盾",
            "stale" => "过时",
            "missing_concept" => "缺页概念",
            _ => self.r#type.as_str(),
        };
        if self.pages.is_empty() {
            format!("语义 lint：{t}")
        } else {
            format!("语义 lint：{t} — {}", self.pages.join(" / "))
        }
    }
}

#[derive(Debug, Deserialize)]
struct LlmIssues {
    #[serde(default)]
    issues: Vec<SemanticIssue>,
}

/// 入队语义 lint 任务（返回 job_id；重复跑安全——检查是只读+review 落库）。
pub async fn enqueue(
    pool: &PgPool,
    lib: Uuid,
    slugs: Option<Vec<String>>,
) -> Result<Uuid, JobError> {
    let tpl = engram_jobs::types::JobTemplate::new("wiki_lint_deep")
        .with_payload(serde_json::json!({ "library_id": lib, "slugs": slugs }));
    let queued = engram_jobs::queue::JobQueue::new(pool.clone())
        .enqueue(tpl)
        .await?;
    Ok(queued.id)
}

/// 语义 lint 任务主体：取范围内页面 → 分批送 LLM → 发现写 review_items。
pub async fn lint_deep_job(
    ctx: &engram_jobs::JobContext,
    llm: &crate::service::LlmRef,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool().clone();
    let payload = &ctx.job.payload.0;
    let lib: Uuid = payload
        .get("library_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 library_id".into()))?;
    let slugs: Option<Vec<String>> = payload.get("slugs").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    });

    // 范围：库内非系统页（log/index/overview 是系统/派生页），slugs 可再过滤
    let pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, title, content FROM wiki_pages \
         WHERE library_id = $1 AND page_type NOT IN ('log','index','overview') \
           AND ($2::text[] IS NULL OR slug = ANY($2)) \
         ORDER BY slug",
    )
    .bind(lib)
    .bind(slugs.as_deref())
    .fetch_all(&pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    if pages.len() < 2 {
        return Ok(serde_json::json!({
            "checked_pages": pages.len(), "batches": 0, "issues": [],
            "note": "页面不足 2 个，无语义检查意义"
        }));
    }

    let mut all_issues: Vec<SemanticIssue> = Vec::new();
    let mut batches = 0usize;

    for chunk in pages.chunks(BATCH) {
        let mut user = String::from("以下是 Wiki 页面集合，请做语义健康检查：\n");
        for (slug, title, content) in chunk {
            let mut body: String = content.chars().take(PAGE_SNIPPET).collect();
            if content.chars().count() > PAGE_SNIPPET {
                body.push('…');
            }
            user.push_str(&format!("\n## [{slug}] {title}\n{body}\n"));
        }
        user.push_str(
            "\n请只报告有把握的问题，输出 JSON：{\"issues\": [{\"type\": \"contradiction|stale|missing_concept\", \"pages\": [\"涉及页 slug\"], \"detail\": \"具体问题（引用双方原句关键部分）\", \"suggestion\": \"建议\"}]}。\n\
             - contradiction：两页对同一事实给出冲突说法\n\
             - stale：某页声明已被另一页的更新信息取代\n\
             - missing_concept：多页反复引用但尚无独立页面的重要概念（pages 留空，slug 候选写进 suggestion）\n\
             没有问题就输出 {\"issues\": []}。不要编造页面 slug。",
        );

        let out = chat_json_retrying(
            ctx,
            llm.as_ref(),
            Purpose::WikiLint,
            &prompts_lint_system(),
            &user,
            ctx.job.id,
        )
        .await?;
        batches += 1;
        let parsed: LlmIssues = serde_json::from_value(out)
            .map_err(|e| JobError::Retryable(format!("语义 lint 输出结构不符: {e}")))?;
        all_issues.extend(parsed.issues);
    }

    // 防幻觉：pages 必须真实存在（missing_concept 除外）
    let known: std::collections::HashSet<String> = pages.iter().map(|(s, ..)| s.clone()).collect();
    all_issues.retain(|it| it.pages.is_empty() || it.pages.iter().all(|p| known.contains(p)));

    let ids = crate::review::create_lint_items(&pool, lib, &all_issues).await?;

    Ok(serde_json::json!({
        "checked_pages": pages.len(),
        "batches": batches,
        "issues": all_issues.len(),
        "review_item_ids": ids,
    }))
}

fn prompts_lint_system() -> String {
    "你是 Wiki 语义健康检查员。你的任务是跨页面比对声明，找出：\
     矛盾（同一事实两页说法冲突）、过时（新信息已取代旧声明但旧页未更新）、\
     重要概念缺页（被反复引用却没有自己的页面）。\
     只报告有把握、有原文依据的问题；宁缺毋滥。只输出合法 JSON。"
        .into()
}
