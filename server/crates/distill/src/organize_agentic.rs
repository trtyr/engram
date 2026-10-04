//! P012：organize agentic 循环——六工具 + finish，模型自主探索与决策。
//!
//! 结构不变量（用户拍板 2026-10-03）：
//! - 删除一律软删（retired_at）——模型无物理删权限；物理解散仅 converge 确定性路径
//! - converge 不交给模型（scenario_converge.rs 保持确定性代码，organize::run 原样）
//! - max_steps=20 写死（非配置项）；成本闸复用 record_llm_call（T016 预算）
//! - 全工具调用落事件流（logs 域）——每步查/改可回放
//!
//! 传输协议：JSON 动作轮转（{tool, args} → 执行 → 结果注入下轮），复用 chat_json
//! 基建（MockLlm 可测，GatewayLlm 无需原生 tool-use）。

use std::collections::HashSet;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::llm_port::{DistillLlm, chat_json_retrying};
use engram_jobs::{JobContext, JobError};

/// 循环步数硬顶（结构不变量，写死不设 env）。
pub const ORGANIZE_MAX_STEPS: usize = 20;

/// agentic 组织产物：touched 场景（链式 persona）+ retired 释放的表述（removed_texts）。
pub struct AgenticOutcome {
    pub touched: Vec<Uuid>,
    pub removed_texts: Vec<String>,
    pub steps: usize,
}

// ---------- 工具执行层（全部复用 repo SQL / T003 归一写法） ----------

/// atoms_pending：散落原子分页（active + 无归属）。
async fn tool_atoms_pending(pool: &PgPool, page: i64) -> Result<Value, JobError> {
    let page = page.max(1);
    let page_size = 30i64;
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, kind, content FROM atoms \
         WHERE status = 'active' AND scenario_id IS NULL \
         ORDER BY created_at LIMIT $1 OFFSET $2",
    )
    .bind(page_size)
    .bind((page - 1) * page_size)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atoms WHERE status = 'active' AND scenario_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({
        "page": page,
        "total": total,
        "atoms": rows.iter().map(|(id, kind, content)| json!({
            "id": id.to_string(), "kind": kind, "content": content,
        })).collect::<Vec<_>>(),
    }))
}

/// scenarios_search：纯向量检索（查询文本现场向量化；P015 FTS 退役）。
async fn tool_scenarios_search(
    pool: &PgPool,
    llm: &dyn DistillLlm,
    job_id: Uuid,
    query: &str,
) -> Result<Value, JobError> {
    // P015：纯向量检索（FTS 退役）——查询文本现场向量化
    let vec_rows: Vec<(Uuid, String, String)> = if let Ok(Some(v)) = llm
        .embed(std::slice::from_ref(&query.to_string()), job_id)
        .await
        .map(|vecs| vecs.first().cloned())
    {
        sqlx::query_as(
            "SELECT id, topic, summary FROM scenarios \
             WHERE retired_at IS NULL AND embedding IS NOT NULL \
             ORDER BY embedding <=> $1 LIMIT 5",
        )
        .bind(pgvector::Vector::from(v.clone()))
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let merged: Vec<Value> = vec_rows
        .into_iter()
        .map(|(id, topic, summary)| {
            json!({"id": id.to_string(), "topic": topic, "summary": summary})
        })
        .collect();
    Ok(json!({
        "query": query,
        "results": merged,
        "hint": "命中场景可 action=scenario_get 看成员全文；相似场景应 update 归并而非另建。"
    }))
}

/// scenario_get：场景详情 + 成员原子全文。
async fn tool_scenario_get(pool: &PgPool, sid: Uuid) -> Result<Value, JobError> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT topic, summary, body FROM scenarios WHERE id = $1 AND retired_at IS NULL",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let Some((topic, summary, body)) = row else {
        return Ok(json!({"error": "场景不存在或已退役", "scenario_id": sid.to_string()}));
    };
    let members: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, kind, content FROM atoms WHERE scenario_id = $1 ORDER BY created_at",
    )
    .bind(sid)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({
        "scenario_id": sid.to_string(),
        "topic": topic, "summary": summary, "body": body,
        "members": members.iter().map(|(id, kind, content)| json!({
            "id": id.to_string(), "kind": kind, "content": content,
        })).collect::<Vec<_>>(),
    }))
}

/// scenario_write：create/update 合一——成员全量语义（真源挂链 + refs 重算，T003 归一）。
async fn tool_scenario_write(
    pool: &PgPool,
    scenario_id: Option<Uuid>,
    topic: &str,
    summary: &str,
    body: &str,
    member_atom_ids: &[Uuid],
) -> Result<Value, JobError> {
    let sid = match scenario_id {
        Some(sid) => {
            // update：成员挂真源（幂等覆盖）→ 重算本场景与失去成员的旧场景缓存
            let old: Vec<Uuid> = sqlx::query_scalar(
                "SELECT DISTINCT scenario_id FROM atoms \
                 WHERE id = ANY($1) AND scenario_id IS NOT NULL AND scenario_id <> $2",
            )
            .bind(member_atom_ids)
            .bind(sid)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            sqlx::query("UPDATE atoms SET scenario_id = $2, updated_at = now() WHERE id = ANY($1)")
                .bind(member_atom_ids)
                .bind(sid)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            let mut affected = old;
            affected.push(sid);
            sqlx::query(
                "UPDATE scenarios SET summary = $2, body = $3, \
                 atom_refs = ( \
                     SELECT COALESCE(jsonb_agg(a.id::text ORDER BY a.created_at), '[]'::jsonb) \
                     FROM atoms a WHERE a.scenario_id = scenarios.id \
                 ), version = version + 1, updated_at = now() \
                 WHERE id = ANY($1) AND retired_at IS NULL",
            )
            .bind(&affected)
            .bind(summary)
            .bind(body)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            json!({"scenario_id": sid.to_string(), "action": "updated", "members": member_atom_ids.len()})
        }
        None => {
            // create：新建场景 + 成员挂真源 + refs 初值
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(topic)
            .bind(summary)
            .bind(body)
            .bind(sqlx::types::Json(
                member_atom_ids
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>(),
            ))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            sqlx::query("UPDATE atoms SET scenario_id = $2, updated_at = now() WHERE id = ANY($1)")
                .bind(member_atom_ids)
                .bind(id)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            json!({"scenario_id": id.to_string(), "action": "created", "members": member_atom_ids.len()})
        }
    };
    Ok(sid)
}

/// scenario_merge：from 成员迁入 into，from 软删（retired）。
async fn tool_scenario_merge(pool: &PgPool, from: Uuid, into: Uuid) -> Result<Value, JobError> {
    if from == into {
        return Ok(json!({"error": "from 与 into 不得相同"}));
    }
    sqlx::query("UPDATE atoms SET scenario_id = $2, updated_at = now() WHERE scenario_id = $1")
        .bind(from)
        .bind(into)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    sqlx::query(
        "UPDATE scenarios SET retired_at = now(), atom_refs = '[]'::jsonb, updated_at = now() \
         WHERE id = $1",
    )
    .bind(from)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    sqlx::query(
        "UPDATE scenarios SET \
         atom_refs = ( \
             SELECT COALESCE(jsonb_agg(a.id::text ORDER BY a.created_at), '[]'::jsonb) \
             FROM atoms a WHERE a.scenario_id = scenarios.id \
         ), version = version + 1, updated_at = now() \
         WHERE id = $1",
    )
    .bind(into)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({"merged_from": from.to_string(), "into": into.to_string(), "from_retired": true}))
}

/// scenario_retire：软删——成员释放回未归组，返回释放的表述（供 persona removed_texts）。
async fn tool_scenario_retire(
    pool: &PgPool,
    sid: Uuid,
    removed_texts: &mut Vec<String>,
) -> Result<Value, JobError> {
    let texts: Vec<(String,)> = sqlx::query_as("SELECT content FROM atoms WHERE scenario_id = $1")
        .bind(sid)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    for (c,) in &texts {
        removed_texts.push(c.clone());
    }
    sqlx::query("UPDATE atoms SET scenario_id = NULL, updated_at = now() WHERE scenario_id = $1")
        .bind(sid)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    let r = sqlx::query(
        "UPDATE scenarios SET retired_at = now(), atom_refs = '[]'::jsonb, updated_at = now() \
         WHERE id = $1 AND retired_at IS NULL",
    )
    .bind(sid)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(json!({
        "scenario_id": sid.to_string(),
        "retired": r.rows_affected() > 0,
        "released_atoms": texts.len(),
        "hint": "软删完成：成员已释放回未归组（下轮 atoms_pending 可见），场景行保留可追溯。"
    }))
}

// ---------- 系统提示词（工具手册 + 组织原则） ----------

fn system_prompt() -> &'static str {
    "你是记忆场景组织者（L1→L2 工匠）。用工具自主探索与决策，把散落原子归入场景。

工具（每轮输出一个 JSON 对象 {\"tool\": …, \"args\": …}）：
- atoms_pending   {page}            散落原子分页（active 且未归属）
- scenarios_search {query}          搜场景（关键词+向量双路）——归并前必查，防另建重复
- scenario_get     {scenario_id}    场景详情含成员全文
- scenario_write   {scenario_id?, topic, summary, body, member_atom_ids[]} 创建/更新场景；member_atom_ids 是全量成员
- scenario_merge   {from_id, into_id} 重复场景合并（from 退役成员迁入）
- scenario_retire  {scenario_id}    软删场景（成员释放回未归组）
- finish           {summary}        交卷（总结本轮组织动作）

组织原则（硬规则）：
1. 同主题不同措辞严禁另建新场景——先 scenarios_search，命中就 update 归并。
2. 冲突内容必须 update 到同一场景（事实以新代旧），不许并存矛盾场景。
3. 推断性原子（非用户明示）content 必带「（推断）」后缀——从原样继承，不改写。
4. 删除只能 scenario_retire（软删），成员会释放回未归组。
5. 不确定归属的原子留在未归组（宁缺毋滥），不要硬塞。
6. 步数有限（20 步），优先处理批量：一次 write 可以收编多个原子。
每轮只输出一个 JSON 动作对象，不要其他文字。"
}

// ---------- 循环器 ----------

/// agentic 组织主循环（T003 接入点：organize::run 主组织段按 settings 开关分派到此）。
pub async fn run_agentic(
    ctx: &JobContext,
    llm: &dyn DistillLlm,
    pending_total: i64,
) -> Result<AgenticOutcome, JobError> {
    let pool = ctx.pool();
    let mut touched: HashSet<Uuid> = HashSet::new();
    let mut removed_texts: Vec<String> = Vec::new();
    let mut history: Vec<Value> = Vec::new();
    let mut steps = 0usize;

    let tool_manual = json!({
        "pending_total": pending_total,
        "note": "散落原子从 atoms_pending 分页查看；组织完成后务必 finish 交卷。",
    });

    while steps < ORGANIZE_MAX_STEPS {
        steps += 1;
        let mut user = json!({
            "task": tool_manual,
            "history": history,
        });
        if history.is_empty() {
            user["task"]["hint"] = json!("先 atoms_pending 看散落原子，再逐场景组织。");
        }
        let user_text =
            serde_json::to_string(&user).map_err(|e| JobError::Retryable(e.to_string()))?;
        // 成本闸在 chat_json_retrying 内（record_llm_call——T016 预算）
        let resp = chat_json_retrying(
            ctx,
            llm,
            engram_llm::types::Purpose::Organize,
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
            "atoms_pending" => {
                let page = args.get("page").and_then(|v| v.as_i64()).unwrap_or(1);
                tool_atoms_pending(pool, page).await?
            }
            "scenarios_search" => {
                let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                tool_scenarios_search(pool, llm, ctx.job.id, q).await?
            }
            "scenario_get" => match parse_uuid(&args, "scenario_id")?.flatten() {
                Some(id) => tool_scenario_get(pool, id).await?,
                None => json!({"error": "scenario_id 缺失或非法"}),
            },
            "scenario_write" => {
                let sid = parse_uuid(&args, "scenario_id")?.flatten();
                let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let members: Vec<Uuid> = args
                    .get("member_atom_ids")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                if topic.is_empty() || summary.is_empty() {
                    json!({"error": "topic/summary 必填"})
                } else {
                    let r = tool_scenario_write(pool, sid, topic, summary, body, &members).await?;
                    if let Some(id) = sid {
                        touched.insert(id);
                    }
                    if let Some(new_id) = r
                        .get("scenario_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok())
                    {
                        touched.insert(new_id);
                    }
                    r
                }
            }
            "scenario_merge" => {
                let from = parse_uuid(&args, "from_id")?.flatten();
                let into = parse_uuid(&args, "into_id")?.flatten();
                match (from, into) {
                    (Some(f), Some(i)) => {
                        touched.insert(i);
                        touched.remove(&f);
                        tool_scenario_merge(pool, f, i).await?
                    }
                    _ => json!({"error": "from_id/into_id 缺失或非法"}),
                }
            }
            "scenario_retire" => {
                let sid = parse_uuid(&args, "scenario_id")?.flatten();
                match sid {
                    Some(id) => {
                        touched.remove(&id);
                        tool_scenario_retire(pool, id, &mut removed_texts).await?
                    }
                    None => json!({"error": "scenario_id 缺失或非法"}),
                }
            }
            "finish" => {
                let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                ctx.emit(
                    &format!("agentic 组织交卷（{steps} 步）：{summary}"),
                    Some(json!({"steps": steps, "touched": touched.len()})),
                )
                .await
                .ok();
                return Ok(AgenticOutcome {
                    touched: touched.into_iter().collect(),
                    removed_texts,
                    steps,
                });
            }
            other => {
                json!({"error": format!("未知工具 \"{other}\"——可用 atoms_pending/scenarios_search/scenario_get/scenario_write/scenario_merge/scenario_retire/finish")})
            }
        };

        ctx.emit(
            &format!("agentic step {steps}: {tool}"),
            Some(json!({"step": steps, "tool": tool, "args": args, "result_excerpt": result.to_string().chars().take(300).collect::<String>()})),
        )
        .await
        .ok();

        history.push(json!({"tool": tool, "args": args, "result": result}));
    }

    // 步数硬顶：不强制作废——已执行的动作已落库，交卷收尾
    ctx.emit(
        &format!("agentic 组织达到步数上限（{ORGANIZE_MAX_STEPS}），按已执行动作收尾"),
        Some(json!({"steps": steps, "touched": touched.len()})),
    )
    .await
    .ok();
    Ok(AgenticOutcome {
        touched: touched.into_iter().collect(),
        removed_texts,
        steps,
    })
}

fn parse_uuid(args: &Value, key: &str) -> Result<Option<Option<Uuid>>, JobError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            Ok(Some(Some(Uuid::parse_str(s).map_err(|_| {
                JobError::Permanent(format!("{key} 不是合法 UUID"))
            })?)))
        }
        Some(_) => Err(JobError::Permanent(format!("{key} 必须是字符串"))),
    }
}
