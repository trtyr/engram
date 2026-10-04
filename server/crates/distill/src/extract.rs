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

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let sessions = claim_pending_sessions(&ctx).await?;
    if sessions.is_empty() {
        // P-B（直写重建死路）：无待蒸馏会话 ≠ 无事可做——直写原子（scenario_id NULL）
        // 也要进场景聚类。organize 自带空输入幂等（收敛扫描 + 未归组原子），空转成本可忽略。
        ctx.enqueue_next(JobTemplate::new("organize_scenarios"))
            .await?;
        return Ok(json!({"session_ids": [], "candidate_ids": [], "chained_organize": true}));
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

    // T014（决策 001）：JEV 保险级联——段级三档路由。哨兵不可用 = 全部直通（降级不阻塞）。
    let mut jev = jev_client_for(&ctx).await;
    let mut guard_stats: (u64, u64, u64) = (0, 0, 0); // (pass, review, reject)

    let mut pending: Vec<PendingAtom> = Vec::new();
    let mut all_relations: Vec<(String, String, String)> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        // 哨兵判定（一次请求两问：guard noul + 归因 choice）；失败 → 本任务起降级直通
        let mut force_review = false;
        if let Some(client) = &jev {
            let seg_text = seg
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            match engram_llm::decisions::guard_segment(client, &seg_text).await {
                Ok(g) => match g.outcome {
                    engram_llm::decisions::GuardOutcome::Reject => {
                        guard_stats.2 += 1;
                        ctx.emit(
                            "JEV 拒绝（跳过精抽）",
                            Some(serde_json::json!({
                                "segment": i + 1,
                                "p": g.p,
                                "reason": g.reason.unwrap_or_else(|| "unknown".into()),
                            })),
                        )
                        .await
                        .ok();
                        continue;
                    }
                    engram_llm::decisions::GuardOutcome::Review => {
                        guard_stats.1 += 1;
                        force_review = true;
                        ctx.emit(
                            "JEV 低置信（产物提级待审）",
                            Some(serde_json::json!({ "segment": i + 1, "p": g.p })),
                        )
                        .await
                        .ok();
                    }
                    engram_llm::decisions::GuardOutcome::Pass => {
                        guard_stats.0 += 1;
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, segment = i + 1, "JEV 哨兵失败——本任务起降级直通");
                    ctx.emit(
                        "JEV 哨兵不可用（降级直通）",
                        Some(serde_json::json!({ "error": e.to_string() })),
                    )
                    .await
                    .ok();
                    jev = None;
                }
            }
        }
        let (atoms, rels) = extract_segment(&ctx, llm.as_ref(), seg, i + 1, total_segments).await?;
        all_relations.extend(rels);
        let mut parsed = parse_atoms(&atoms, &turn_map, &session_sensitive);
        if force_review {
            for p in &mut parsed {
                p.force_review = true;
            }
        }
        pending.extend(parsed);
    }
    if jev.is_some() {
        ctx.emit(
            "JEV 保险级联",
            Some(serde_json::json!({
                "segments": total_segments,
                "pass": guard_stats.0,
                "review": guard_stats.1,
                "reject": guard_stats.2,
            })),
        )
        .await
        .ok();
    }

    let candidate_ids = persist_atoms(&ctx, llm.as_ref(), pending).await?;
    persist_relations(&ctx, &all_relations).await;
    mark_sessions_done(&ctx, &session_ids).await?;

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

    // P015：在线仲裁退役——抽取直落 active + 向量化；判重/取代移交离线整理。
    // 组织仍链式触发（无仲裁接棒后由 extract 直接链），离线整理上线后移交。
    ctx.enqueue_next(JobTemplate::new("organize_scenarios"))
        .await?;
    Ok(json!({"session_ids": session_ids, "candidate_ids": candidate_ids}))
}

/// JEV 哨兵解析（T014）：settings 配置 + ctx.cipher → 可用客户端；
/// 未启用/未配置/无 cipher → None（调用方全部直通）。
async fn jev_client_for(ctx: &JobContext) -> Option<engram_llm::decisions::JevClient> {
    // 无 cipher = 未部署哨兵（常态，静默直通不噪音）
    let cipher = ctx.cipher.as_ref()?;
    let cfg: engram_llm::decisions::JevConfig =
        engram_storage::repo::settings::get_json(ctx.pool(), engram_llm::decisions::SETTINGS_KEY)
            .await
            .unwrap_or_default();
    // 配置了但解析失败（坏 key_enc 等）= 异常态，必须可见（T020 精神：降级可观测）
    match engram_llm::decisions::resolve(&cfg, cipher, reqwest::Client::new()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "JEV 配置解析失败——降级直通");
            ctx.emit(
                "JEV 哨兵不可用（降级直通）",
                Some(serde_json::json!({ "error": e.to_string(), "stage": "resolve" })),
            )
            .await
            .ok();
            None
        }
    }
}

/// 单段抽取：一次 LLM 调用 + 段级事件留痕（空段显式确认「无持久洞察」）。
async fn extract_segment(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    seg: &[SegmentLine],
    index: usize,
    total: usize,
) -> Result<(serde_json::Value, Vec<(String, String, String)>), JobError> {
    let user = seg
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Extract,
        &prompts::extract_system(),
        &user,
        ctx.job.id,
    )
    .await?;
    let seg_count = out
        .get("atoms")
        .and_then(|a| a.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if seg_count == 0 {
        ctx.emit(
            &format!("分段抽取 {index}/{total}：无持久洞察（显式确认）"),
            None,
        )
        .await
        .ok();
    } else {
        ctx.emit(
            &format!("分段抽取 {index}/{total}：{seg_count} 条候选"),
            None,
        )
        .await
        .ok();
    }
    Ok((out.clone(), parse_relations(&out)))
}

/// 3. 落库：嵌入 + 插入候选原子 + 实体挂链（失败不阻断主链）。
async fn persist_atoms(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    mut pending: Vec<PendingAtom>,
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
    let pool = ctx.pool();
    let texts: Vec<String> = pending.iter().map(|p| p.content.clone()).collect();
    let embeddings = llm.embed(&texts, ctx.job.id).await?;
    let mut candidate_ids = Vec::with_capacity(pending.len());
    for (i, p) in pending.into_iter().enumerate() {
        let id = Uuid::now_v7();
        // P015：直落 active（无候选态/无待审）——判重与取代由离线整理负责
        let needs_review = false;
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, occurred_at, valid_until, sensitive, embedding, tsv, strength, source_kind)
             VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8, $9, $10, to_tsvector('simple', $11), $12, 'agent_inferred')",
        )
        .bind(id)
        .bind(&p.kind)
        .bind(&p.content)
        .bind(p.confidence)
        .bind(sqlx::types::Json(&p.refs))
        .bind(needs_review)
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
        .bind(engram_search::tokenize::tsv_text(&p.content))
        .bind(&p.strength)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        for (name, kind) in &p.entities {
            if let Err(e) = link_entity(pool, id, name, kind).await {
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
    .fetch_optional(pool)
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
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
            match archived {
                Some(eid) => {
                    engram_storage::repo::memory::revive_entity(pool, eid)
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
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                    sqlx::query_scalar(
                        "SELECT id FROM entities WHERE lower(btrim(name)) = lower(btrim($1)) AND kind = $2 \
                         AND merged_into IS NULL AND archived_at IS NULL",
                    )
                    .bind(name)
                    .bind(kind)
                    .fetch_one(pool)
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
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    // 复活语义已废除（审计五驳）：近似查询只命中活体（merged_into/archived_at 双排除）——
    // eid 恒为活体；归档/墓碑行不再被挂原子、不再被清 archived_at。
    // 抽取要「并」的是活档，不是唤醒死档；同名检测与合并 = 命中活体即复用其 id（幂等）。
    Ok(())
}

/// 关系落库：实体已在挂链里 upsert，name→id 解析后建关系（best-effort，不阻断主链）。
async fn persist_relations(ctx: &JobContext, relations: &[(String, String, String)]) {
    let pool = ctx.pool();
    for (from, to, rel_type) in relations {
        let fid = lookup_entity(pool, from).await;
        let tid = lookup_entity(pool, to).await;
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
        .execute(pool)
        .await
        {
            tracing::warn!(error = %e, "关系落库失败（不影响主链）");
        }
    }
}

/// T005：关系 lookup 与挂链口径统一（trim/lower + 子串归并同款）。
pub async fn lookup_entity(pool: &sqlx::PgPool, name: &str) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM entities \
         WHERE merged_into IS NULL AND archived_at IS NULL \
           AND (lower(btrim(name)) = lower(btrim($1)) OR position(lower(name) in lower($1)) > 0) \
         ORDER BY (lower(btrim(name)) = lower(btrim($1))) DESC, length(name) DESC, created_at ASC \
         LIMIT 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 4. 会话标记完成。
async fn mark_sessions_done(ctx: &JobContext, session_ids: &[Uuid]) -> Result<(), JobError> {
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = ANY($1)")
        .bind(session_ids)
        .execute(ctx.pool())
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}
