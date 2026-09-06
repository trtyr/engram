//! extract：L0 会话 → 候选 L1 原子。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

/// 宽容 ISO8601 解析：完整 RFC3339 或 date-only（"2026-09-02" → 当日零点 UTC）。
/// LLM 输出的时间五花八门，这里只接受这两种最常见形态，其余静默丢弃（时间字段可选）。
fn parse_iso(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.into());
    }
    chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc))
}

/// 原始会话行（extract 内部用）。
struct SessionRow {
    id: Uuid,
    agent: String,
    content: serde_json::Value,
    sensitive: bool,
    metadata: serde_json::Value,
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    // 1. 认领待蒸馏会话（processing 中防重复认领）。
    //    v2 修复（H-A2）：metadata.distill=off 的会话**永久豁免**——off 是会话级语义，
    //    不再被后续任何 extract 任务的 pending 全量扫描顺带蒸掉。
    let sessions: Vec<SessionRow> =
        sqlx::query_as::<_, (Uuid, String, serde_json::Value, bool, serde_json::Value)>(
            "UPDATE raw_sessions SET distill_status = 'processing' \
         WHERE distill_status = 'pending' AND COALESCE(metadata->>'distill','') <> 'off' \
         RETURNING id, agent, content, sensitive, metadata",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .into_iter()
        .map(|(id, agent, content, sensitive, metadata)| SessionRow {
            id,
            agent,
            content,
            sensitive,
            metadata,
        })
        .collect();

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
            let revert = sqlx::query(
                "UPDATE raw_sessions SET distill_status = 'pending' \
                 WHERE id = ANY($1) AND distill_status = 'processing'",
            )
            .bind(&session_ids)
            .execute(ctx.pool())
            .await;
            if let Err(re) = revert {
                tracing::error!(error = %re, "会话状态回滚失败（将滞留 processing，需人工处理）");
            }
            Err(e)
        }
    }
}

/// 分段字符预算（B1 覆盖率）：段内拼多轮，超预算开新段；单段一次 LLM 调用。
const SEGMENT_CHARS: usize = 6000;

struct SegmentLine {
    text: String,
    is_header: bool,
}

async fn run_claimed(
    ctx: JobContext,
    llm: LlmRef,
    sessions: Vec<SessionRow>,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();
    // 会话敏感标记映射：任一轮次来源会话标 sensitive → 产物继承敏感
    let session_sensitive: std::collections::HashMap<Uuid, bool> =
        sessions.iter().map(|s| (s.id, s.sensitive)).collect();

    // 2. 构建行列表（会话头 + 全局编号轮次）→ 按预算贪心分段
    //    B1：一批会话全拼一个 prompt 时，超上下文的中间轮次会被模型静默丢弃；
    //    分段后每段独立调用，段级事件留痕，覆盖率可审计。
    let mut lines: Vec<SegmentLine> = Vec::new();
    let mut turn_map: Vec<Uuid> = Vec::new(); // 轮次编号(1-based) → session_id
    for s in &sessions {
        let is_import = s
            .metadata
            .get("source")
            .and_then(|v| v.as_str())
            .map(|src| src == "import")
            .unwrap_or(false);
        let import_hint = if is_import {
            "（批量导入的历史——对方的话是素材，不是用户本人的记忆）"
        } else {
            ""
        };
        lines.push(SegmentLine {
            text: format!("— 会话 {}（agent: {}）{}—", s.id, s.agent, import_hint),
            is_header: true,
        });
        if let Some(turns) = s.content.as_array() {
            for t in turns {
                let speaker = t.get("speaker").and_then(|v| v.as_str()).unwrap_or("?");
                let text = t.get("text").and_then(|v| v.as_str()).unwrap_or("");
                turn_map.push(s.id);
                lines.push(SegmentLine {
                    text: format!("[{}] {}: {}", turn_map.len(), speaker, text),
                    is_header: false,
                });
            }
        }
    }

    // 贪心打包：会话头不落单（开新段时若末行是头，连带迁去新段）
    let mut segments: Vec<Vec<SegmentLine>> = vec![Vec::new()];
    let mut used = 0usize;
    for line in lines {
        let len = line.text.len() + 1;
        if !segments.last().unwrap().is_empty() && used + len > SEGMENT_CHARS {
            let mut new_seg: Vec<SegmentLine> = Vec::new();
            // 头不落单：上一段末行若是会话头，迁移到新段首
            if segments
                .last()
                .unwrap()
                .last()
                .map(|l| l.is_header)
                .unwrap_or(false)
                && let Some(header) = segments.last_mut().unwrap().pop()
            {
                new_seg.push(header);
            }
            segments.push(new_seg);
            used = 0;
        }
        used += len;
        segments.last_mut().unwrap().push(line);
    }
    segments.retain(|s| !s.is_empty());
    let total_segments = segments.len();

    // 3. 逐段抽取（每段独立 LLM 调用，段级事件留痕；空段显式确认「无持久洞察」）
    /// 段内抽取产物（候选原子 + 实体挂链素材）
    struct PendingAtom {
        kind: String,
        content: String,
        confidence: f32,
        refs: serde_json::Value,
        entities: Vec<(String, String)>,
        occurred_at: Option<chrono::DateTime<chrono::Utc>>,
        valid_until: Option<chrono::DateTime<chrono::Utc>>,
        sensitive: bool,
    }
    let mut texts: Vec<String> = Vec::new();
    let mut pending: Vec<PendingAtom> = Vec::new();
    let mut all_relations: Vec<(String, String, String)> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let user = seg
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let out = crate::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            engram_llm::types::Purpose::Extract,
            &prompts::extract_system(),
            &user,
            ctx.job.id,
        )
        .await?;

        let atoms = out
            .get("atoms")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        // 关系抽取：顶层 relations（from/to 用规范称呼，rel_type 限定五类）
        if let Some(rels) = out.get("relations").and_then(|r| r.as_array()) {
            for r in rels {
                let from = r
                    .get("from")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string());
                let to = r
                    .get("to")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string());
                let rel_type = r
                    .get("rel_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("related_to");
                if let (Some(f), Some(t)) = (from, to)
                    && !f.is_empty()
                    && !t.is_empty()
                    && f != t
                    && matches!(
                        rel_type,
                        "member_of" | "located_in" | "works_on" | "part_of" | "related_to"
                    )
                {
                    all_relations.push((f, t, rel_type.to_string()));
                }
            }
        }
        let seg_count = atoms.len();
        for a in &atoms {
            let kind = a
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("fact")
                .to_string();
            let content = a
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if content.is_empty() || content.chars().count() > 120 {
                continue;
            }
            let confidence = a
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.7)
                .clamp(0.0, 1.0) as f32;
            let refs = a
                .get("turn_refs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|n| n.as_u64().map(|i| (i as usize).checked_sub(1)))
                        .flatten()
                        .filter_map(|i| turn_map.get(i).copied())
                        .map(|sid| json!({"session_id": sid}))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            // 议题二：相对时间已由 prompt（今天锚）让 LLM 换算成 ISO8601；这里宽容解析
            let occurred_at = a
                .get("occurred_at")
                .and_then(|v| v.as_str())
                .and_then(parse_iso);
            let valid_until = a
                .get("valid_until")
                .and_then(|v| v.as_str())
                .and_then(parse_iso);
            // 实体：该条记忆的主角（人/项目/主题/群组）——name+kind 归一后落库挂链
            let entities: Vec<(String, String)> = a
                .get("entities")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let name = e.get("name")?.as_str()?.trim().to_string();
                            let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("topic");
                            if name.is_empty()
                                || name.chars().count() > 60
                                || !matches!(
                                    kind,
                                    "person" | "project" | "topic" | "group" | "place"
                                )
                            {
                                return None;
                            }
                            Some((name, kind.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            // 会话敏感继承：任一轮次来源会话标 sensitive → 产物 sensitive
            let sensitive = refs.iter().any(|r| {
                r.get("session_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<Uuid>().ok())
                    .map(|sid| *session_sensitive.get(&sid).unwrap_or(&false))
                    .unwrap_or(false)
            });
            texts.push(content.clone());
            pending.push(PendingAtom {
                kind,
                content,
                confidence,
                refs: json!(refs),
                entities,
                occurred_at,
                valid_until,
                sensitive,
            });
        }
        if seg_count == 0 {
            ctx.emit(
                &format!(
                    "分段抽取 {}/{}：无持久洞察（显式确认）",
                    i + 1,
                    total_segments
                ),
                None,
            )
            .await
            .ok();
        } else {
            ctx.emit(
                &format!(
                    "分段抽取 {}/{}：{} 条候选",
                    i + 1,
                    total_segments,
                    seg_count
                ),
                None,
            )
            .await
            .ok();
        }
    }

    let mut candidate_ids = Vec::new();
    if !pending.is_empty() {
        let embeddings = llm.embed(&texts, ctx.job.id).await?;
        for (i, p) in pending.into_iter().enumerate() {
            let PendingAtom {
                kind,
                content,
                confidence,
                refs,
                entities,
                occurred_at,
                valid_until,
                sensitive,
            } = p;
            let id = Uuid::now_v7();
            let needs_review = confidence < 0.55;
            sqlx::query(
                "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, occurred_at, valid_until, sensitive, embedding, tsv)
                 VALUES ($1, $2, $3, $4, 'candidate', $5, $6, $7, $8, $9, $10, to_tsvector('simple', $11))",
            )
            .bind(id)
            .bind(&kind)
            .bind(&content)
            .bind(confidence)
            .bind(sqlx::types::Json(&refs))
            .bind(needs_review)
            .bind(occurred_at)
            .bind(valid_until)
            .bind(sensitive)
            // B2：嵌入缺失或全零（上游异常）一律置 NULL——零向量会污染余弦近邻
            .bind(
                embeddings
                    .get(i)
                    .filter(|v| v.iter().any(|&x| x != 0.0))
                    .map(|v| pgvector::Vector::from(v.clone())),
            )
            .bind(engram_search::tokenize::tsv_text(&content))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            // 实体挂链：同名同类活体复用（部分唯一索引），否则新建；失败不阻断蒸馏主链。
            // v2 修复（N5）：新建前先做**名字包含**近似归并——LLM 对同一实体常给出措辞
            // 繁简不同的称呼（「星云」/「星云项目」），互为子串且同 kind 即视为同一实体，
            // 挂到既有实体上，不再各建一个空档案。
            for (name, ekind) in &entities {
                let link = async {
                    let similar: Option<Uuid> = sqlx::query_scalar(
                        "SELECT id FROM entities WHERE kind = $2 AND merged_into IS NULL \
                         AND (position($1 in name) > 0 OR position(name in $1) > 0) LIMIT 1",
                    )
                    .bind(name)
                    .bind(ekind)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                    let eid: Uuid = match similar {
                        Some(eid) => eid,
                        None => {
                            sqlx::query(
                                "INSERT INTO entities (id, name, kind) VALUES ($1, $2, $3) \
                                 ON CONFLICT DO NOTHING",
                            )
                            .bind(Uuid::now_v7())
                            .bind(name)
                            .bind(ekind)
                            .execute(pool)
                            .await
                            .map_err(|e| e.to_string())?;
                            sqlx::query_scalar(
                                "SELECT id FROM entities WHERE name = $1 AND kind = $2 AND merged_into IS NULL",
                            )
                            .bind(name)
                            .bind(ekind)
                            .fetch_one(pool)
                            .await
                            .map_err(|e| e.to_string())?
                        }
                    };
                    sqlx::query(
                        "INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2) \
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(id)
                    .bind(eid)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                    // 实体复活：蒸馏重新挂链被归档的同名实体 → 清标记回到可见层
                    sqlx::query("UPDATE entities SET archived_at = NULL WHERE id = $1")
                        .bind(eid)
                        .execute(pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok::<(), String>(())
                };
                if let Err(e) = link.await {
                    tracing::warn!(error = %e, entity = %name, "实体挂链失败（不影响原子产出）");
                }
            }
            candidate_ids.push(id);
        }
    }

    // 关系落库：实体已在挂链里 upsert，name→id 解析后建关系（best-effort，不阻断主链）
    for (from, to, rel_type) in &all_relations {
        let fid: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM entities WHERE name = $1 AND merged_into IS NULL ORDER BY created_at DESC LIMIT 1",
        )
        .bind(from)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let tid: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM entities WHERE name = $1 AND merged_into IS NULL ORDER BY created_at DESC LIMIT 1",
        )
        .bind(to)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        if let (Some(fid), Some(tid)) = (fid, tid)
            && let Err(e) = sqlx::query(
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

    // 4. 会话标记完成
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = ANY($1)")
        .bind(&session_ids)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

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

    // 5. 链式入队仲裁
    if !candidate_ids.is_empty() {
        ctx.enqueue_next(
            JobTemplate::new("arbitrate_atoms")
                .with_payload(json!({"candidate_ids": candidate_ids})),
        )
        .await?;
    }

    Ok(json!({"session_ids": session_ids, "candidate_ids": candidate_ids}))
}
