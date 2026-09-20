//! arbitrate：候选 × 既有相似 → 新增 / 去重 / 矛盾取代。
//!
//! 分步（架构治理 2026-09-20，纯搬移 + 抽函数）：取数 → 相似检索与提示构建 → LLM 裁决
//! → 落库应用 → 漏判兜底 → 事件与链式入队。每步一个函数（≤60 行），纯逻辑（裁决解析）
//! 独立成 `parse_verdicts`，可不依赖库与 LLM 单测。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

struct AtomRow {
    id: Uuid,
    content: String,
}

/// LLM 对单条候选的裁决（已过滤幻觉 id）。
#[derive(Debug, PartialEq)]
struct Verdict {
    candidate_id: Uuid,
    disposition: String,
    target_id: Option<Uuid>,
}

/// 仲裁结果三态。
#[derive(Default)]
struct Outcome {
    promoted: Vec<Uuid>,
    duplicates: Vec<Uuid>,
    superseded: Vec<(Uuid, Uuid)>, // (new, old)
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    let candidates = fetch_candidates(&ctx).await?;
    if candidates.is_empty() {
        return Ok(json!({"promoted": [], "duplicates": [], "superseded": []}));
    }

    let (user, no_similar) = build_arbitrate_prompt(pool, &candidates).await?;

    let mut outcome = Outcome::default();
    promote_atoms(pool, &no_similar).await?;
    outcome.promoted.extend(no_similar.iter().copied());

    // 有相似的交 LLM 仲裁；全无相似则跳过 LLM 调用
    if candidates.len() > no_similar.len() {
        let verdicts = arbitrate_with_llm(&ctx, llm.as_ref(), &user, &candidates).await?;
        apply_verdicts(pool, &verdicts, &mut outcome).await?;
        let leftover = promote_leftovers(pool, &candidates).await?;
        outcome.promoted.extend(leftover);
    }

    emit_summary(&ctx, &outcome).await;
    enqueue_organize(&ctx, &outcome).await?;
    Ok(report(&outcome))
}

/// 1. 取候选：payload 指定 id；未指定则兜底取全部 candidate（上限 200）。
async fn fetch_candidates(ctx: &JobContext) -> Result<Vec<AtomRow>, JobError> {
    let pool = ctx.pool();
    let ids: Vec<Uuid> = ctx
        .job
        .payload
        .0
        .get("candidate_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();

    let rows: Vec<(Uuid, String)> = if ids.is_empty() {
        sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, content FROM atoms WHERE status = 'candidate' ORDER BY created_at LIMIT 200",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
    } else {
        sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, content FROM atoms WHERE id = ANY($1) AND status = 'candidate'",
        )
        .bind(&ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
    };

    Ok(rows
        .into_iter()
        .map(|(id, content)| AtomRow { id, content })
        .collect())
}

/// 2. 相似检索 + 提示构建：返回（提示文本，无相似候选 id）。
async fn build_arbitrate_prompt(
    pool: &sqlx::PgPool,
    candidates: &[AtomRow],
) -> Result<(String, Vec<Uuid>), JobError> {
    let mut user = String::new();
    let mut no_similar: Vec<Uuid> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let similar = find_similar(pool, c).await?;
        writeln!(user, "候选[{}]: id={} 内容={}", i, c.id, c.content).ok();
        if similar.is_empty() {
            no_similar.push(c.id);
        } else {
            for (sid, scontent) in similar {
                writeln!(user, "  既有 id={} 内容={}", sid, scontent).ok();
            }
        }
    }
    Ok((user, no_similar))
}

/// 单条候选的相似既有原子：ANN ∪ FTS 并集（去重保序，cap 8 防提示过长）。
async fn find_similar(pool: &sqlx::PgPool, c: &AtomRow) -> Result<Vec<(Uuid, String)>, JobError> {
    let has_emb: bool = sqlx::query_scalar("SELECT embedding IS NOT NULL FROM atoms WHERE id = $1")
        .bind(c.id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut similar: Vec<(Uuid, String)> = Vec::new();
    if has_emb {
        let ann: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, content FROM atoms \
             WHERE status = 'active' AND embedding IS NOT NULL \
             ORDER BY embedding <=> (SELECT embedding FROM atoms WHERE id = $1) \
             LIMIT 5",
        )
        .bind(c.id)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        similar.extend(ann);
    }
    if engram_search::tokenize::has_query_tokens(&c.content) {
        // FTS 补位：写入与查询同源 jieba 分词——无嵌入原子（种子/历史直插）由此可达
        let fts: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, content FROM atoms, to_tsquery('simple', $1) q \
             WHERE status = 'active' AND tsv @@ q \
             ORDER BY ts_rank(tsv, q) DESC LIMIT 5",
        )
        .bind(engram_search::tokenize::tsv_query_smart(&c.content, 3))
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        similar.extend(fts);
    }
    let mut seen = HashSet::new();
    similar.retain(|(sid, _)| seen.insert(*sid));
    similar.truncate(8);
    Ok(similar)
}

/// 3. LLM 裁决：调用 + 解析（幻觉 id 在解析层过滤）。
async fn arbitrate_with_llm(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    user: &str,
    candidates: &[AtomRow],
) -> Result<Vec<Verdict>, JobError> {
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Arbitrate,
        &prompts::arbitrate_system(),
        user,
        ctx.job.id,
    )
    .await?;
    let valid_ids: HashSet<Uuid> = candidates.iter().map(|c| c.id).collect();
    Ok(parse_verdicts(&out, &valid_ids))
}

/// 纯函数：把 LLM 输出解析为裁决列表（跳过缺 id / 幻觉 id；disposition 缺省视为 new）。
fn parse_verdicts(out: &Value, valid_ids: &HashSet<Uuid>) -> Vec<Verdict> {
    let verdicts = out
        .get("verdicts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parsed = Vec::with_capacity(verdicts.len());
    for v in verdicts {
        let Some(cid) = v
            .get("candidate_id")
            .and_then(|x| x.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        if !valid_ids.contains(&cid) {
            continue; // 防幻觉 id
        }
        let disposition = v
            .get("disposition")
            .and_then(|x| x.as_str())
            .unwrap_or("new")
            .to_string();
        let target_id = v
            .get("target_id")
            .and_then(|x| x.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        parsed.push(Verdict {
            candidate_id: cid,
            disposition,
            target_id,
        });
    }
    parsed
}

/// 4. 落库应用：按裁决逐条落库（duplicate 归档 / contradicts 取代 / 其他转正）。
async fn apply_verdicts(
    pool: &sqlx::PgPool,
    verdicts: &[Verdict],
    outcome: &mut Outcome,
) -> Result<(), JobError> {
    for v in verdicts {
        match v.disposition.as_str() {
            "duplicate" => apply_duplicate(pool, v, outcome).await?,
            "contradicts" => {
                if let Some(t) = v.target_id {
                    // 候选转正 + 旧条 superseded
                    promote_atoms(pool, std::slice::from_ref(&v.candidate_id)).await?;
                    mark_superseded(pool, v.candidate_id, t).await?;
                    outcome.superseded.push((v.candidate_id, t));
                    outcome.promoted.push(v.candidate_id);
                } else {
                    // 无 target 的 contradicts 视为 new
                    promote_atoms(pool, std::slice::from_ref(&v.candidate_id)).await?;
                    outcome.promoted.push(v.candidate_id);
                }
            }
            _ => {
                promote_atoms(pool, std::slice::from_ref(&v.candidate_id)).await?;
                outcome.promoted.push(v.candidate_id);
            }
        }
    }
    Ok(())
}

/// 5. LLM 漏判兜底：仍滞留 candidate 的一律转正（不能让 candidate 滞留）。
async fn promote_leftovers(
    pool: &sqlx::PgPool,
    candidates: &[AtomRow],
) -> Result<Vec<Uuid>, JobError> {
    let leftover: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM atoms WHERE id = ANY($1) AND status = 'candidate'")
            .bind(candidates.iter().map(|c| c.id).collect::<Vec<_>>())
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    promote_atoms(pool, &leftover).await?;
    Ok(leftover)
}

// ---------- 落库原语 ----------

async fn promote_atoms(pool: &sqlx::PgPool, ids: &[Uuid]) -> Result<(), JobError> {
    for id in ids {
        sqlx::query("UPDATE atoms SET status = 'active' WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    Ok(())
}

async fn archive_candidate(pool: &sqlx::PgPool, id: Uuid) -> Result<(), JobError> {
    sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE id = $1 AND status = 'candidate'",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

async fn mark_superseded(pool: &sqlx::PgPool, new_id: Uuid, old_id: Uuid) -> Result<(), JobError> {
    sqlx::query(
        "UPDATE atoms SET status = 'superseded', superseded_by = $1, updated_at = now() WHERE id = $2 AND status = 'active'",
    )
    .bind(new_id)
    .bind(old_id)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

async fn bump_hit_count(pool: &sqlx::PgPool, id: Uuid) -> Result<(), JobError> {
    sqlx::query("UPDATE atoms SET hit_count = hit_count + 1, updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

// ---------- 6. 事件与链式入队 ----------

async fn emit_summary(ctx: &JobContext, outcome: &Outcome) {
    ctx.emit(
        &format!(
            "仲裁：转正 {} / 去重 {} / 取代 {}",
            outcome.promoted.len(),
            outcome.duplicates.len(),
            outcome.superseded.len()
        ),
        None,
    )
    .await
    .ok();
}

/// 链式入队组织（仅有转正时）。
async fn enqueue_organize(ctx: &JobContext, outcome: &Outcome) -> Result<(), JobError> {
    if !outcome.promoted.is_empty() {
        ctx.enqueue_next(
            JobTemplate::new("organize_scenarios")
                .with_payload(json!({"atom_ids": outcome.promoted})),
        )
        .await?;
    }
    Ok(())
}

fn report(outcome: &Outcome) -> serde_json::Value {
    json!({
        "promoted": outcome.promoted,
        "duplicates": outcome.duplicates,
        "superseded": outcome.superseded.iter().map(|(n, o)| json!({"new": n, "old": o})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn uu(n: u8) -> Uuid {
        Uuid::from_u128(n as u128)
    }

    #[test]
    fn parse_verdicts_filters_hallucinated_and_missing_ids() {
        let valid: HashSet<Uuid> = [uu(1), uu(2)].into_iter().collect();
        let out = json!({"verdicts": [
            {"candidate_id": uu(1).to_string(), "disposition": "duplicate", "target_id": uu(9).to_string()},
            {"candidate_id": uu(7).to_string(), "disposition": "new"},   // 幻觉 id → 丢
            {"disposition": "new"},                                      // 缺 id → 丢
            {"candidate_id": uu(2).to_string()},                         // 缺 disposition → 默认 new
            {"candidate_id": "不是 UUID", "disposition": "new"},          // 坏 id → 丢
        ]});
        let got = parse_verdicts(&out, &valid);
        assert_eq!(got.len(), 2, "只应保留两条合法裁决：{got:?}");
        assert_eq!(got[0].candidate_id, uu(1));
        assert_eq!(got[0].disposition, "duplicate");
        assert_eq!(got[0].target_id, Some(uu(9)));
        assert_eq!(got[1].candidate_id, uu(2));
        assert_eq!(got[1].disposition, "new");
        assert_eq!(got[1].target_id, None);
    }

    #[test]
    fn parse_verdicts_tolerates_missing_or_malformed_payload() {
        let valid: HashSet<Uuid> = [uu(1)].into_iter().collect();
        assert!(
            parse_verdicts(&json!({}), &valid).is_empty(),
            "缺 verdicts 键应得空"
        );
        assert!(parse_verdicts(&json!({"verdicts": "不是数组"}), &valid).is_empty());
        assert!(parse_verdicts(&json!({"verdicts": []}), &valid).is_empty());
    }
}

/// duplicate 裁决：归档候选 + superseded_by 指向既有条（B6 不物理删除，保留审计与恢复能力）；
/// 无 target 的异常裁决仅归档，不动任何既有条。
async fn apply_duplicate(
    pool: &sqlx::PgPool,
    v: &Verdict,
    outcome: &mut Outcome,
) -> Result<(), JobError> {
    // B6：判重不再物理删除——归档 + superseded_by 指向既有条，
    // 保留审计与恢复能力（LLM 误判时可追溯），UI 的 active 过滤天然屏蔽
    if let Some(t) = v.target_id {
        sqlx::query(
            "UPDATE atoms SET status = 'archived', superseded_by = $2, updated_at = now() \
                         WHERE id = $1 AND status = 'candidate'",
        )
        .bind(v.candidate_id)
        .bind(t)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        bump_hit_count(pool, t).await?;
    } else {
        // 无 target 的 duplicate（异常裁决）：仅归档保留，不动任何既有条
        archive_candidate(pool, v.candidate_id).await?;
    }
    outcome.duplicates.push(v.candidate_id);
    Ok(())
}
