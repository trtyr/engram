//! 离线整理 Agent（P015）：定期扫全量原子 → 判重合并/归档过时 → 维护画像活文档。
//! JSON 协议循环（P012 organize_agentic 同款）：{tool, args} → 执行 → 结果注入 history → finish。
//!
//! 职责边界：在线写入路径只管落库（extract 直落 active）；判重、取代、归档、
//! 画像演化全部在本 Agent 的离线巡逻里完成。

use crate::llm_port::LlmRef;
use crate::llm_port::{DistillLlm, chat_json_retrying};
use engram_jobs::{JobContext, JobError};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

/// 步数硬顶：全量巡逻 + 多轮编辑比单次组织更费步，但结构不变量不是配置项。
pub const MAINTAIN_MAX_STEPS: usize = 30;

#[derive(Debug, Clone)]
pub struct MaintainOutcome {
    pub merged: usize,
    pub archived: usize,
    pub persona_edited: bool,
    pub steps: usize,
}

fn system_prompt() -> &'static str {
    "你是一个严谨的记忆整理官。你定期巡逻整个记忆库（原子记忆），负责两件事：\
判重合并、归档过时；并维护一份描述用户的画像文档（Markdown）。
直接输出 JSON，不要解释、不要分析过程。

## 原子治理
1. 判重：两条原子**语义等价**（同一事实的不同措辞、同一事件的重复陈述）才算重复——
   用 atom_merge 合并：keep 选信息更全/更早的一条，merge_ids 放其余。
   互补细节不算重复（都留）；仅有时间差异不算重复（都留）。
2. 矛盾处理：两条原子互相矛盾（旧说法 vs 新说法）——归档旧的那条（atom_archive 带 reason），
   保留新的。不确定哪新哪旧就都不动。
3. 归档过时：只归档**有明确更新证据**的原子（库里有新条覆盖了它）。拿不准就不动。
4. 工作节奏：先用 atoms_recent 总览，再对可疑主题 atoms_search 深挖；一次 atom_merge/atom_archive
   处理一组；治理若干组后再做画像。

## 画像文档维护
1. 画像是描述用户的一份 Markdown 人物侧写（身份/偏好/关系/工作/设备/长期项目等，自定结构）。
2. 先 persona_doc_read 读现状，再用本次巡逻中查到的原子**增量修正**：补新条目、改过时描述、删失效项。
3. 禁止推倒重写：新版本必须以上一版为基础逐处修订（文档不存在才允许初建）。
4. 只依据原子写画像——每条画像内容都应能追溯到原子证据；原子库里没有的不要编。

## 收尾
全部完成后 finish（summary 里报告合并/归档/画像编辑的计数）。步数有限（30），别在同一主题上反复横跳。"
}

/// 工具：tsv 全文搜 active 原子。
async fn tool_atoms_search(pool: &PgPool, q: &str, limit: i64) -> Result<Value, JobError> {
    let rows: Vec<(uuid::Uuid, String, String, f32, Option<String>)> = sqlx::query_as(
        "SELECT id, kind, content, confidence, occurred_at::text FROM atoms \
         WHERE status = 'active' AND tsv @@ plainto_tsquery('simple', $1) \
         ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(q)
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({
        "count": rows.len(),
        "atoms": rows.iter().map(|(id, kind, content, conf, occurred)| json!({
            "id": id.to_string(), "kind": kind, "content": content,
            "confidence": conf, "occurred_at": occurred,
        })).collect::<Vec<_>>(),
    }))
}

/// 工具：最近的 active 原子（浏览入口）。
async fn tool_atoms_recent(pool: &PgPool, limit: i64) -> Result<Value, JobError> {
    let rows: Vec<(uuid::Uuid, String, String, f32)> = sqlx::query_as(
        "SELECT id, kind, content, confidence FROM atoms \
         WHERE status = 'active' ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({
        "count": rows.len(),
        "atoms": rows.iter().map(|(id, kind, content, conf)| json!({
            "id": id.to_string(), "kind": kind, "content": content, "confidence": conf,
        })).collect::<Vec<_>>(),
    }))
}

/// 工具：语义重复合并——victims 归档并挂取代指针指向 keep，source_refs 并入 keep。
async fn tool_atom_merge(
    pool: &PgPool,
    keep_id: uuid::Uuid,
    merge_ids: &[uuid::Uuid],
) -> Result<Value, JobError> {
    let victims: Vec<uuid::Uuid> = merge_ids
        .iter()
        .filter(|m| **m != keep_id)
        .cloned()
        .collect();
    if victims.is_empty() {
        return Ok(json!({"merged": 0, "note": "merge_ids 为空或与 keep 相同"}));
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    let res = sqlx::query(
        "UPDATE atoms a SET status = 'archived', superseded_by = $2, updated_at = now() \
         FROM (SELECT id, source_refs FROM atoms WHERE id = ANY($1) AND status = 'active') v \
         WHERE a.id = v.id RETURNING v.source_refs",
    )
    .bind(&victims)
    .bind(keep_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let extra: Vec<Value> = res
        .iter()
        .filter_map(|r| r.try_get::<serde_json::Value, _>("source_refs").ok())
        .collect();
    if !extra.is_empty() {
        sqlx::query(
            "UPDATE atoms SET source_refs = source_refs || $2::jsonb, updated_at = now() WHERE id = $1",
        )
        .bind(keep_id)
        .bind(sqlx::types::Json(&extra))
        .execute(&mut *tx)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({"merged": res.len(), "keep": keep_id.to_string()}))
}

/// 工具：过时归档（无取代对象）。
async fn tool_atom_archive(pool: &PgPool, id: uuid::Uuid) -> Result<Value, JobError> {
    let r = sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE id = $1 AND status = 'active' RETURNING id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({"archived": r.is_some(), "id": id.to_string()}))
}

/// 任务入口薄壳（chain 注册签名对齐 organize.rs）。
pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<Value, JobError> {
    run_maintain(&ctx, llm.as_ref()).await
}

pub async fn run_maintain(ctx: &JobContext, llm: &dyn DistillLlm) -> Result<Value, JobError> {
    let pool = ctx.pool();
    let mut merged = 0usize;
    let mut archived = 0usize;
    let mut persona_edited = false;
    let mut history: Vec<Value> = Vec::new();
    let mut steps = 0usize;

    let atom_total: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE status = 'active'")
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let tool_manual = json!({
        "active_atoms_total": atom_total,
        "note": "atoms_recent 总览 → atoms_search 深挖可疑主题 → atom_merge/atom_archive 治理 → persona_doc_read/persona_doc_edit 维护画像 → finish 交卷。",
    });

    while steps < MAINTAIN_MAX_STEPS {
        steps += 1;
        let mut user = json!({ "task": tool_manual, "history": history });
        if history.is_empty() {
            user["task"]["hint"] = json!("从 atoms_recent 开始总览。");
        }
        let user_text =
            serde_json::to_string(&user).map_err(|e| JobError::Retryable(e.to_string()))?;
        let resp = chat_json_retrying(
            ctx,
            llm,
            engram_llm::types::Purpose::Consolidate,
            system_prompt(),
            &user_text,
            ctx.job.id,
        )
        .await?;
        let tool = resp
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let args = resp.get("args").cloned().unwrap_or_else(|| json!({}));

        let result = match tool.as_str() {
            "atoms_search" => {
                let q = args
                    .get("q")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
                tool_atoms_search(pool, &q, limit).await?
            }
            "atoms_recent" => {
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
                tool_atoms_recent(pool, limit).await?
            }
            "atom_merge" => {
                let keep = args
                    .get("keep_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());
                let merges: Vec<uuid::Uuid> = args
                    .get("merge_ids")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                match keep {
                    Some(k) if !merges.is_empty() => {
                        let r = tool_atom_merge(pool, k, &merges).await?;
                        if let Some(n) = r.get("merged").and_then(|v| v.as_u64()) {
                            merged += n as usize;
                        }
                        r
                    }
                    _ => json!({"error": "需要 keep_id 与非空 merge_ids"}),
                }
            }
            "atom_archive" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());
                match id {
                    Some(id) => {
                        let r = tool_atom_archive(pool, id).await?;
                        if r.get("archived").and_then(|v| v.as_bool()).unwrap_or(false) {
                            archived += 1;
                        }
                        r
                    }
                    None => json!({"error": "需要 id"}),
                }
            }
            "persona_doc_read" => {
                let doc = engram_storage::repo::memory::persona_doc_get(pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                match doc {
                    Some(d) => json!({"version": d.version, "content": d.content}),
                    None => {
                        json!({"version": 0, "content": "", "note": "画像文档尚未建立——首次巡逻请依据查到的原子初建"})
                    }
                }
            }
            "persona_doc_edit" => {
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if content.trim().is_empty() {
                    json!({"error": "content 为空"})
                } else {
                    let summary = args.get("summary").and_then(|v| v.as_str());
                    let ver =
                        engram_storage::repo::memory::persona_doc_save(pool, &content, summary)
                            .await
                            .map_err(|e| JobError::Retryable(e.to_string()))?;
                    persona_edited = true;
                    json!({"saved_version": ver})
                }
            }
            "finish" => {
                ctx.emit(
                    &format!(
                        "整理巡逻完成：合并 {merged}、归档 {archived}、画像{}",
                        if persona_edited {
                            "已更新"
                        } else {
                            "未变"
                        }
                    ),
                    None,
                )
                .await
                .ok();
                return Ok(json!({
                    "merged": merged, "archived": archived,
                    "persona_edited": persona_edited, "steps": steps,
                    "summary": args.get("summary").cloned().unwrap_or(json!("")),
                }));
            }
            other => json!({"error": format!("未知工具：{other}")}),
        };

        history.push(json!({ "tool": tool, "args": args, "result": result }));
    }

    ctx.emit(
        &format!("整理巡逻达到步数上限（{MAINTAIN_MAX_STEPS}），按已执行动作收尾"),
        None,
    )
    .await
    .ok();
    Ok(json!({
        "merged": merged, "archived": archived,
        "persona_edited": persona_edited, "steps": steps,
    }))
}
