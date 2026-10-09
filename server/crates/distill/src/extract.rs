//! extract：L0 会话 → 候选 L1 原子。
//!
//! 分步（架构治理 2026-09-20）：认领会话（失败回滚）→ 分段 → 逐段 LLM 抽取
//! → 落库（含实体挂链）→ 关系落库 → 会话标记完成 → 链式入队仲裁。
//! 纯逻辑（分段打包、解析、白名单）在 `extract_model`，可独立单测。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::extract_model::{
    PendingAtom, SegmentLine, SessionRow, build_segments, parse_atoms, parse_relations,
};
use crate::llm_port::LlmRef;
use crate::prompts;

use engram_llm::types::{ChatMessage, Purpose, ToolDef};

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let sessions = claim_pending_sessions(&ctx).await?;
    if sessions.is_empty() {
        // P015 场景层退役：无待蒸馏会话即无事可做（整理归离线节律，不再链式）。
        return Ok(json!({"session_ids": [], "candidate_ids": []}));
    }
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();

    // 失败回滚：任何 Err 都把认领会话退回 pending（重试不丢数据）
    match run_claimed(ctx.clone(), llm, sessions).await {
        Ok(v) => Ok(v),
        Err(e) => {
            revert_sessions(&ctx, &session_ids).await;
            Err(e)
        }
    }
}

/// 1. 认领待蒸馏会话（processing 中防重复认领）。
///
/// v2 修复（H-A2）：`metadata.distill=off` 的会话**永久豁免**——off 是会话级语义，
/// 不再被后续任何 extract 任务的 pending 全量扫描顺带蒸掉。
/// 认领上限（T006）：一次最多认领 N 个 pending 会话，剩余留给下一批（满批自续）。
/// 性能参数（非正确性不变量），写死避免误配成全量抢占。
const EXTRACT_CLAIM_BATCH: i64 = 50;

async fn claim_pending_sessions(ctx: &JobContext) -> Result<Vec<SessionRow>, JobError> {
    // T006：LIMIT 分批 + FOR UPDATE SKIP LOCKED（多 worker 并发安全，不再全量抢占）
    let rows = sqlx::query_as::<_, (Uuid, String, serde_json::Value, bool, serde_json::Value)>(
        "UPDATE raw_sessions SET distill_status = 'processing' \
         WHERE id IN ( \
             SELECT id FROM raw_sessions \
             WHERE distill_status = 'pending' AND COALESCE(metadata->>'distill','') <> 'off' \
             ORDER BY created_at ASC LIMIT $1 \
             FOR UPDATE SKIP LOCKED \
         ) \
         RETURNING id, agent, content, sensitive, metadata",
    )
    .bind(EXTRACT_CLAIM_BATCH)
    .fetch_all(ctx.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|(id, agent, content, sensitive, metadata)| SessionRow {
            id,
            agent,
            content,
            sensitive,
            metadata,
        })
        .collect())
}

async fn revert_sessions(ctx: &JobContext, session_ids: &[Uuid]) {
    let revert = sqlx::query(
        "UPDATE raw_sessions SET distill_status = 'pending' \
         WHERE id = ANY($1) AND distill_status = 'processing'",
    )
    .bind(session_ids)
    .execute(ctx.pool())
    .await;
    if let Err(re) = revert {
        tracing::error!(error = %re, "会话状态回滚失败（将滞留 processing，需人工处理）");
    }
}

async fn run_claimed(
    ctx: JobContext,
    llm: LlmRef,
    sessions: Vec<SessionRow>,
) -> Result<serde_json::Value, JobError> {
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();
    // 会话敏感标记映射：任一轮次来源会话标 sensitive → 产物继承敏感
    let session_sensitive: HashMap<Uuid, bool> =
        sessions.iter().map(|s| (s.id, s.sensitive)).collect();

    let (segments, turn_map) = build_segments(&sessions);
    let total_segments = segments.len();

    let mut pending: Vec<PendingAtom> = Vec::new();
    let mut all_relations: Vec<(String, String, String)> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let (atoms, rels) = extract_segment(
            &ctx,
            llm.as_ref(),
            seg,
            i + 1,
            total_segments,
            &turn_map,
            &session_sensitive,
        )
        .await?;
        all_relations.extend(rels);
        pending.extend(atoms);
    }

    // P019-M1：落库+会话标记同事务——旧序 persist 后若 mark_sessions_done 失败，
    // 队列重试会重新抽取并把同一批原子再插一遍（重复落库且立即向量化）。
    // 嵌入/LLM 调用都在事务外完成，事务只覆盖纯 DB 写（短事务）。
    let mut tx = ctx
        .pool()
        .begin()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    let candidate_ids = persist_atoms(&ctx, llm.as_ref(), pending, &mut tx).await?;
    persist_relations(&all_relations, &mut tx).await;
    mark_sessions_done(&session_ids, &mut tx).await?;
    tx.commit()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // T006：满批自续——认领满额说明可能仍有积压，链式投下一批（批间独立，无幂等键）
    if sessions.len() as i64 >= EXTRACT_CLAIM_BATCH {
        ctx.enqueue_next(
            JobTemplate::new("extract_atoms").with_payload(json!({"reason": "batch-continue"})),
        )
        .await?;
    }

    ctx.emit(
        &format!(
            "抽取 {} 会话 → {} 候选",
            session_ids.len(),
            candidate_ids.len()
        ),
        None,
    )
    .await
    .ok();

    // P015：抽取直落 active + 向量化即完成——判重/归档/画像归离线整理 Agent
    // （节律每天巡逻 + 手动触发），写入链不再串行接棒。
    Ok(json!({"session_ids": session_ids, "candidate_ids": candidate_ids}))
}

/// Agent 工具循环单段轮数上限（防失控；正常 1-3 轮收敛）。
const MAX_AGENT_ROUNDS: usize = 12;

/// 抽取 Agent 的工具面：模型通过调用工具写入产物，不再输出自由文本 JSON。
fn extract_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "save_atoms".into(),
            description: "提交抽取到的候选原子记忆。可多次调用，每次任意条；全部提交完后停止调工具即可".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "atoms": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": {
                                    "type": "string",
                                    "enum": ["preference", "fact", "decision", "event", "insight", "correction", "failure", "convention"]
                                },
                                "content": {"type": "string"},
                                "confidence": {"type": "number"},
                                "strength": {"type": "string", "enum": ["fact", "inference", "assumption"]},
                                "turn_refs": {"type": "array", "items": {"type": "integer"}},
                                "occurred_at": {"type": ["string", "null"]},
                                "valid_until": {"type": ["string", "null"]},
                                "entities": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "name": {"type": "string"},
                                            "kind": {"type": "string", "enum": ["person", "project", "topic", "group", "place"]}
                                        },
                                        "required": ["name", "kind"]
                                    }
                                }
                            },
                            "required": ["kind", "content", "confidence", "strength", "turn_refs"]
                        }
                    }
                },
                "required": ["atoms"]
            }),
        },
        ToolDef {
            name: "add_relation".into(),
            description: "提交实体间关系（可选）：仅在对话明确表达实体关系时调用".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "relations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": {"type": "string"},
                                "to": {"type": "string"},
                                "rel_type": {"type": "string", "enum": ["member_of", "located_in", "works_on", "part_of", "related_to"]}
                            },
                            "required": ["from", "to", "rel_type"]
                        }
                    }
                },
                "required": ["relations"]
            }),
        },
        ToolDef {
            name: "no_insight".into(),
            description: "显式确认整段对话没有值得长期记住的内容（纯闲聊/事务性操作/全部是项目内部事实或瞬态读数）".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {"type": "string"}
                },
                "required": ["reason"]
            }),
        },
    ]
}

/// 工具分发：结构化参数直接进白名单解析（parse_atoms/parse_relations），
/// 不存在自由文本 JSON 解析路径。返回值回填给模型（tool 消息）。
fn dispatch_tool(
    name: &str,
    arguments: &str,
    turn_map: &[Uuid],
    session_sensitive: &HashMap<Uuid, bool>,
    atoms: &mut Vec<PendingAtom>,
    relations: &mut Vec<(String, String, String)>,
) -> String {
    let args: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(e) => return format!("错误：工具参数不是合法 JSON：{e}。请用正确的 JSON 参数重试"),
    };
    match name {
        "save_atoms" => {
            let parsed = parse_atoms(&args, turn_map, session_sensitive);
            let n = parsed.len();
            atoms.extend(parsed);
            format!("已接收 {n} 条候选原子（本批累计 {} 条）", atoms.len())
        }
        "add_relation" => {
            let parsed = parse_relations(&args);
            let n = parsed.len();
            relations.extend(parsed);
            format!("已接收 {n} 条关系")
        }
        "no_insight" => {
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            format!("已确认：本段无持久洞察（{reason}）")
        }
        other => {
            format!("错误：未知工具 {other}，可用工具：save_atoms / add_relation / no_insight")
        }
    }
}

/// 单段抽取（Agent 工具循环）：模型自主调工具写入产物，
/// 停止调用工具（纯文本收尾）或调 no_insight 即完成本段。
/// 每轮调用过 record_llm_call 预算闸（T007/T016），完整 I/O 经 GatewayLlm 记账。
async fn extract_segment(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    seg: &[SegmentLine],
    index: usize,
    total: usize,
    turn_map: &[Uuid],
    session_sensitive: &HashMap<Uuid, bool>,
) -> Result<(Vec<PendingAtom>, Vec<(String, String, String)>), JobError> {
    let user = seg
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut messages = vec![
        ChatMessage::system(prompts::extract_agent_system()),
        ChatMessage::user(user),
    ];
    let tools = extract_tools();
    let mut atoms: Vec<PendingAtom> = Vec::new();
    let mut relations: Vec<(String, String, String)> = Vec::new();
    let mut rounds = 0usize;

    loop {
        // T007/T016：每轮模型调用记账——超任务预算 → BudgetExceeded（failed 可 revive）
        ctx.record_llm_call()?;
        let resp = llm
            .chat_tools(Purpose::Extract, &messages, &tools, ctx.job.id)
            .await?;
        let calls = match resp.tool_calls {
            Some(c) if !c.is_empty() => c,
            _ => {
                // 模型纯文本收尾 = 本段完成
                break;
            }
        };
        rounds += 1;
        messages.push(ChatMessage::assistant_with_tool_calls(
            resp.content.clone(),
            calls.clone(),
        ));
        let mut declared_no_insight = false;
        for c in calls {
            if c.name.trim() == "no_insight" {
                declared_no_insight = true;
            }
            let result = dispatch_tool(
                c.name.trim(),
                &c.arguments,
                turn_map,
                session_sensitive,
                &mut atoms,
                &mut relations,
            );
            messages.push(ChatMessage::tool_result(c.id, result));
        }
        if declared_no_insight || rounds >= MAX_AGENT_ROUNDS {
            if !declared_no_insight {
                tracing::warn!(index, rounds, "agent 循环达轮数上限，强制收尾");
            }
            break;
        }
    }

    if atoms.is_empty() {
        ctx.emit(
            &format!("分段抽取 {index}/{total}：无持久洞察（显式确认）"),
            None,
        )
        .await
        .ok();
    } else {
        ctx.emit(
            &format!(
                "分段抽取 {index}/{total}：{} 条候选（{rounds} 轮工具调用）",
                atoms.len()
            ),
            None,
        )
        .await
        .ok();
    }
    Ok((atoms, relations))
}

/// 3. 落库：嵌入 + 插入候选原子 + 实体挂链（失败不阻断主链）。
///
/// 在调用方事务内执行（conn），与 mark_sessions_done 同事务防重放重复落库。
async fn persist_atoms(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    mut pending: Vec<PendingAtom>,
    conn: &mut sqlx::PgConnection,
) -> Result<Vec<Uuid>, JobError> {
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    // P015：宁缺毋滥——低置信直接丢弃（无待审通道）；准入语义在抽取 prompt 内（v9 worth_memorizing）
    let dropped = pending.iter().filter(|p| p.confidence < 0.55).count();
    if dropped > 0 {
        ctx.emit(&format!("低置信丢弃 {dropped} 条（conf<0.55）"), None)
            .await
            .ok();
    }
    pending.retain(|p| p.confidence >= 0.55);
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<String> = pending.iter().map(|p| p.content.clone()).collect();
    let embeddings = llm.embed(&texts, ctx.job.id).await?;
    let mut candidate_ids = Vec::with_capacity(pending.len());
    for (i, p) in pending.into_iter().enumerate() {
        let id = Uuid::now_v7();
        // P015：直落 active（无候选态/无待审）——判重与取代由离线整理负责
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, occurred_at, valid_until, sensitive, embedding, strength, source_kind)
             VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8, $9, $10, 'agent_inferred')",
        )
        .bind(id)
        .bind(&p.kind)
        .bind(&p.content)
        .bind(p.confidence)
        .bind(sqlx::types::Json(&p.refs))
        .bind(p.occurred_at)
        .bind(p.valid_until)
        .bind(p.sensitive)
        // B2：嵌入缺失或全零（上游异常）一律置 NULL——零向量会污染余弦近邻
        .bind(
            embeddings
                .get(i)
                .filter(|v| v.iter().any(|&x| x != 0.0))
                .map(|v| pgvector::Vector::from(v.clone())),
        )
        .bind(&p.strength)
        .execute(&mut *conn)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        for (name, kind) in &p.entities {
            if let Err(e) = link_entity_conn(conn, id, name, kind).await {
                tracing::warn!(error = %e, entity = %name, "实体挂链失败（不影响原子产出）");
            }
        }
        candidate_ids.push(id);
    }
    Ok(candidate_ids)
}

/// 实体挂链：同名同类活体复用（部分唯一索引），否则新建。
/// v2 修复（N5）：新建前先做**名字包含**近似归并——LLM 对同一实体常给出措辞
/// 繁简不同的称呼（「星云」/「星云项目」），互为子串且同 kind 即视为同一实体，
/// 挂到既有实体上，不再各建一个空档案。
pub async fn link_entity(
    pool: &sqlx::PgPool,
    atom_id: Uuid,
    name: &str,
    kind: &str,
) -> Result<(), String> {
    let mut conn = pool.acquire().await.map_err(|e| e.to_string())?;
    link_entity_conn(&mut conn, atom_id, name, kind).await
}

async fn link_entity_conn(
    conn: &mut sqlx::PgConnection,
    atom_id: Uuid,
    name: &str,
    kind: &str,
) -> Result<(), String> {
    // T005：误吞修复——只认「旧名完整出现在新名里」（如「王小明」复用「小王」档），
    // 去掉「新名在旧名内」方向（「云」不再吞进「星云」）；精确命中优先，其次最长旧名。
    let similar: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM entities WHERE kind = $2 AND merged_into IS NULL AND archived_at IS NULL \
         AND (lower(btrim(name)) = lower(btrim($1)) OR position(lower(name) in lower($1)) > 0) \
         ORDER BY (lower(btrim(name)) = lower(btrim($1))) DESC, length(name) DESC, created_at ASC \
         LIMIT 1",
    )
    .bind(name)
    .bind(kind)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    let eid: Uuid = match similar {
        Some(eid) => eid,
        None => {
            // T008（Q002 改判）：活体无同名 → 查归档档同名——复活延续旧档案
            // （summary/revision 不沉归档态）；墓碑（merged_into）永不复活。
            let archived: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM entities WHERE kind = $2 AND archived_at IS NOT NULL AND merged_into IS NULL \
                 AND (lower(btrim(name)) = lower(btrim($1)) OR position(lower(name) in lower($1)) > 0) \
                 ORDER BY (lower(btrim(name)) = lower(btrim($1))) DESC, length(name) DESC, created_at ASC \
                 LIMIT 1",
            )
            .bind(name)
            .bind(kind)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| e.to_string())?;
            match archived {
                Some(eid) => {
                    // P019-M1：内联 revive（原 repo::revive_entity 收 &PgPool）——
                    // 本函数运行在调用方事务内，不能跨连接逃逸事务。
                    sqlx::query(
                        "UPDATE entities SET archived_at = NULL \
                         WHERE id = $1 AND archived_at IS NOT NULL AND merged_into IS NULL",
                    )
                    .bind(eid)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| e.to_string())?;
                    eid
                }
                None => {
                    sqlx::query(
                        "INSERT INTO entities (id, name, kind) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                    )
                    .bind(Uuid::now_v7())
                    .bind(name)
                    .bind(kind)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| e.to_string())?;
                    sqlx::query_scalar(
                        "SELECT id FROM entities WHERE lower(btrim(name)) = lower(btrim($1)) AND kind = $2 \
                         AND merged_into IS NULL AND archived_at IS NULL",
                    )
                    .bind(name)
                    .bind(kind)
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(|e| e.to_string())?
                }
            }
        }
    };
    sqlx::query(
        "INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(atom_id)
    .bind(eid)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    // 复活语义已废除（审计五驳）：近似查询只命中活体（merged_into/archived_at 双排除）——
    // eid 恒为活体；归档/墓碑行不再被挂原子、不再被清 archived_at。
    // 抽取要「并」的是活档，不是唤醒死档；同名检测与合并 = 命中活体即复用其 id（幂等）。
    Ok(())
}

/// 关系落库：实体已在挂链里 upsert，name→id 解析后建关系（best-effort，不阻断主链）。
/// 在调用方事务内执行。
async fn persist_relations(relations: &[(String, String, String)], conn: &mut sqlx::PgConnection) {
    for (from, to, rel_type) in relations {
        let fid = lookup_entity_conn(conn, from).await;
        let tid = lookup_entity_conn(conn, to).await;
        let (Some(fid), Some(tid)) = (fid, tid) else {
            continue;
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) \
             VALUES ($1, $2, $3, $4, 1, 'distill') \
             ON CONFLICT (from_id, to_id, rel_type) DO UPDATE SET weight = entity_relations.weight + 1, updated_at = now()",
        )
        .bind(Uuid::now_v7())
        .bind(fid)
        .bind(tid)
        .bind(rel_type)
        .execute(&mut *conn)
        .await
        {
            tracing::warn!(error = %e, "关系落库失败（不影响主链）");
        }
    }
}

/// T005：关系 lookup 与挂链口径统一（trim/lower + 子串归并同款）。
pub async fn lookup_entity(pool: &sqlx::PgPool, name: &str) -> Option<Uuid> {
    let mut conn = pool.acquire().await.ok()?;
    lookup_entity_conn(&mut conn, name).await
}

/// P019-M1：事务内变体。
async fn lookup_entity_conn(conn: &mut sqlx::PgConnection, name: &str) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM entities \
         WHERE merged_into IS NULL AND archived_at IS NULL \
           AND (lower(btrim(name)) = lower(btrim($1)) OR position(lower(name) in lower($1)) > 0) \
         ORDER BY (lower(btrim(name)) = lower(btrim($1))) DESC, length(name) DESC, created_at ASC \
         LIMIT 1",
    )
    .bind(name)
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten()
}

/// 4. 会话标记完成（在调用方事务内，与原子落库同 commit）。
async fn mark_sessions_done(
    session_ids: &[Uuid],
    conn: &mut sqlx::PgConnection,
) -> Result<(), JobError> {
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = ANY($1)")
        .bind(session_ids)
        .execute(&mut *conn)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}
