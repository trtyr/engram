//! 记忆域服务：L0 写入/触发、检索、上下文包、L1 治理、L3 画像视图。

use engram_jobs::types::Job;
use engram_jobs::{JobQueue, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::tokenize;
use engram_search::{SearchHit, search_atoms, search_scenarios};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// 记忆域错误（api 层转 ApiError）。
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
    #[error("LLM 未配置（检索退化为全文通道）: {0}")]
    LlmNotConfigured(String),
}

impl From<sqlx::Error> for MemoryError {
    fn from(e: sqlx::Error) -> Self {
        MemoryError::Storage(e.to_string())
    }
}

// ---------- DTO（api 直接复用，utoipa schema） ----------

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SessionDto {
    pub id: Uuid,
    pub agent: String,
    #[schema(value_type = Object)]
    pub content: serde_json::Value,
    pub distill_status: String,
    /// 会话级敏感标记：蒸馏产物自动继承
    pub sensitive: bool,
    pub created_at: DateTime<Utc>,
    /// 会话元数据（source=import 标记批量导入的历史；蒸馏据此过滤对方观点）
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AtomDto {
    pub id: Uuid,
    pub kind: String,
    pub content: String,
    pub confidence: f32,
    pub status: String,
    pub superseded_by: Option<Uuid>,
    pub needs_review: bool,
    /// P3 隐私标记：医疗/感情/财务类——默认不进检索与 context_pack，reveal 才可见
    pub sensitive: bool,
    pub hit_count: i32,
    pub scenario_id: Option<Uuid>,
    /// 事件时间（extract 以当天为锚把相对时间解析成绝对；created_at 只是记录时间）
    pub occurred_at: Option<DateTime<Utc>>,
    /// 有效期（到期事件可过滤/降权）
    pub valid_until: Option<DateTime<Utc>>,
    #[schema(value_type = Object)]
    pub source_refs: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScenarioDto {
    pub id: Uuid,
    pub topic: String,
    pub summary: String,
    #[schema(value_type = Object)]
    pub atom_refs: serde_json::Value,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
}

/// 原子改写留痕（编辑能力：旧值 + 谁改的）。append-only，随原子级联删除。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AtomRevision {
    pub id: Uuid,
    pub atom_id: Uuid,
    pub old_content: String,
    pub old_kind: String,
    pub old_confidence: f32,
    pub edited_by: String,
    pub created_at: DateTime<Utc>,
}

/// 实体摘要改写留痕（圈子强化：手编档案的轻量历史，复用原子 revisions 模式）。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct EntityRevision {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub old_summary: String,
    pub edited_by: String,
    pub created_at: DateTime<Utc>,
}

/// 实体关系（迁移 0021）：有向类型化关系，图升级成知识图谱。
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct EntityRelationDto {
    pub id: Uuid,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub rel_type: String,
    pub weight: i32,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 全局记忆时间轴事件（原子/场景/实体按时间倒序合并）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TimelineEvent {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    /// atom | scenario | entity
    pub kind: String,
    pub content: String,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct PersonaVersion {
    pub id: Uuid,
    pub aspect: String,
    pub content: String,
    #[schema(value_type = Object)]
    pub evidence_refs: serde_json::Value,
    pub version: i32,
    pub prompt_version: Option<String>,
    pub manually_edited: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContextPack {
    /// L3：画像分面（当前版本，全量）
    pub persona: Vec<PersonaVersion>,
    /// L2：相关/最近场景
    pub scenarios: Vec<ScenarioDto>,
    /// L1：补充原子
    pub atoms: Vec<AtomDto>,
    /// 实体透镜：用户世界里的人/项目/主题（有 query 按相关，无 query 按密度头部）
    pub entities: Vec<EntityDto>,
    /// 待人审项（≤5 条）——AI 在对话中顺口确认后 atom-patch 回写
    pub pending_review: Vec<AtomDto>,
    pub meta: ContextMeta,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContextMeta {
    pub chars_used: usize,
    pub truncated: bool,
    pub query: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SearchResponse {
    /// 实体命中（主角先行——搜人名/项目名先给实体再给相关原子）
    pub entities: Vec<SearchHit>,
    pub l1: Vec<SearchHit>,
    pub l2: Vec<SearchHit>,
    pub l3: Vec<PersonaVersion>,
    pub query: String,
}

// ---------- 实体（记忆星系） ----------

/// 实体类型（迁移 0015 CHECK 枚举）。
pub const ENTITY_KINDS: [&str; 5] = ["person", "project", "topic", "group", "place"];

/// 实体关系类型（迁移 0021 CHECK 枚举）。方向：from --rel_type--> to。
pub const REL_TYPES: [&str; 5] = [
    "member_of",
    "located_in",
    "works_on",
    "part_of",
    "related_to",
];

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct EntityDto {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub summary: String,
    pub atom_count: i64,
    pub manually_edited: Option<bool>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EntityDetail {
    pub entity: EntityDto,
    pub atoms: Vec<AtomDto>,
    pub scenarios: Vec<ScenarioDto>,
    /// 共现邻居：与当前实体共享原子的其他实体（按共现次数降序，最多 20）
    pub neighbors: Vec<EntityDto>,
    /// 类型化关系（有向）：本实体作为 from 或 to 的关系
    pub relations: Vec<EntityRelationDto>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct GraphEdge {
    pub a: Uuid,
    pub b: Uuid,
    pub weight: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EntityGraph {
    pub nodes: Vec<EntityDto>,
    /// 共现边：同一原子同时关联的两个实体（weight = 共同原子数）
    pub edges: Vec<GraphEdge>,
    /// 类型化关系（有向）：图升级成知识图谱的关系边
    pub relations: Vec<EntityRelationDto>,
}

/// 记忆域缺失向量统计（重嵌修复入口）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EmbeddingStatus {
    pub atoms_missing: i64,
    pub scenarios_missing: i64,
}

// ---------- 服务 ----------

#[derive(Clone)]
pub struct MemoryService {
    pool: PgPool,
    queue: JobQueue,
    registry: ProviderRegistry,
    /// 防抖窗口（秒）
    pub debounce_secs: i64,
}

/// deep purge 确认短语（canonical 常量，API 层引用）。
pub const PURGE_CONFIRM_PHRASE: &str = "清空记忆库";

/// 单轮 text 上限（SEC-B，2026-09-03）：防超长文本整轮灌进会话爆蒸馏 token；
/// 长文档应走知识域 /wiki/upload 分块摄取。
pub const TURN_TEXT_MAX_CHARS: usize = 50_000;

/// M-1/SEC-B（2026-09-03）：轮次逐条校验——speaker 合法、text 非空且有上限。
/// 此前空 text 轮次被原样落库（进蒸馏浪费 LLM 调用）、超长轮次无界 accepted。
fn validate_turns(arr: &[serde_json::Value]) -> Result<(), MemoryError> {
    for (i, t) in arr.iter().enumerate() {
        let speaker = t.get("speaker").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(speaker, "user" | "assistant") {
            return Err(MemoryError::BadRequest(format!(
                "第 {} 轮 speaker 必须是 user/assistant（得到「{speaker}」）",
                i + 1
            )));
        }
        let text = t.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim().is_empty() {
            return Err(MemoryError::BadRequest(format!(
                "第 {} 轮 text 不能为空（空轮次进蒸馏只会浪费 LLM 调用）",
                i + 1
            )));
        }
        let n = text.chars().count();
        if n > TURN_TEXT_MAX_CHARS {
            return Err(MemoryError::BadRequest(format!(
                "第 {} 轮 text 超长（{n} 字 > 上限 {TURN_TEXT_MAX_CHARS} 字）——长文档请走 /wiki/upload 分块摄取",
                i + 1
            )));
        }
    }
    Ok(())
}

/// deep purge 核心（供 MemoryService 与 API 的 deep_purge 定时 job 复用）。
pub async fn purge_deep_pool(pool: &sqlx::PgPool) -> Result<serde_json::Value, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM raw_sessions), \
            (SELECT count(*) FROM atoms), \
            (SELECT count(*) FROM entities WHERE merged_into IS NULL), \
            (SELECT count(*) FROM scenarios), \
            (SELECT count(*) FROM persona_aspects)",
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "TRUNCATE atom_entities, entities, persona_aspects, scenarios, atoms, raw_sessions CASCADE",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(serde_json::json!({
        "sessions": counts.0, "atoms": counts.1, "entities": counts.2,
        "scenarios": counts.3, "persona": counts.4,
    }))
}

impl MemoryService {
    pub fn new(pool: PgPool, registry: ProviderRegistry) -> Self {
        Self {
            queue: JobQueue::new(pool.clone()),
            pool,
            registry,
            debounce_secs: 30,
        }
    }

    // ---------- L0 ----------

    /// 写 L0 会话并按策略触发蒸馏。
    pub async fn write_session(
        &self,
        agent: &str,
        turns: serde_json::Value,
        distill: &str,
        sensitive: bool,
    ) -> Result<SessionDto, MemoryError> {
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("会话至少一轮".into()));
        }
        validate_turns(arr)?;
        let id = Uuid::now_v7();
        let row = sqlx::query_as::<_, SessionDto>(
            "INSERT INTO raw_sessions (id, agent, content, sensitive) VALUES ($1, $2, $3, $4) RETURNING *",
        )
        .bind(id)
        .bind(agent)
        .bind(sqlx::types::Json(&turns))
        .bind(sensitive)
        .fetch_one(&self.pool)
        .await?;

        match distill {
            "auto" => {
                engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                    .await
                    .ok();
            }
            "manual" => {
                self.queue
                    .enqueue(
                        JobTemplate::new("extract_atoms").with_payload(json!({"reason": "manual"})),
                    )
                    .await
                    .ok();
            }
            _ => {}
        }
        Ok(row)
    }

    /// 批量导入历史对话为会话（phase-2）：JSONL（每行 {role, content}）或纯文本（空行分段）。
    /// metadata.source = "import"——蒸馏据此感知「导入的历史，对方的话是素材不是用户事实」。
    pub async fn import_session(
        &self,
        agent: &str,
        content: &str,
        format: &str,
        distill: &str,
    ) -> Result<SessionDto, MemoryError> {
        let turns = Self::parse_import(content, format)?;
        // M-1/SEC-B 同口径：导入的 turns 也过校验（jsonl 行 content 为空同样挡）
        if let Some(arr) = turns.as_array() {
            validate_turns(arr)?;
        }
        let id = Uuid::now_v7();
        let row = sqlx::query_as::<_, SessionDto>(
            "INSERT INTO raw_sessions (id, agent, content, metadata) VALUES ($1, $2, $3, $4) RETURNING *",
        )
        .bind(id)
        .bind(agent)
        .bind(sqlx::types::Json(&turns))
        .bind(serde_json::json!({"source": "import"}))
        .fetch_one(&self.pool)
        .await?;
        match distill {
            "auto" => {
                engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                    .await
                    .ok();
            }
            "manual" => {
                self.queue
                    .enqueue(
                        JobTemplate::new("extract_atoms").with_payload(json!({"reason": "import"})),
                    )
                    .await
                    .ok();
            }
            _ => {}
        }
        Ok(row)
    }

    /// 解析导入文本 → turns（[{speaker, text}]）。jsonl：每行 {role, content}；text：空行分段交替。
    fn parse_import(content: &str, format: &str) -> Result<serde_json::Value, MemoryError> {
        let turns: Vec<serde_json::Value> = match format {
            "jsonl" => {
                let mut out = Vec::new();
                for (i, line) in content.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let v: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                        MemoryError::BadRequest(format!(
                            "第 {} 行不是合法 JSON：{}（格式：每行 {{\"role\":\"user\"|\"assistant\",\"content\":\"...\"}}）",
                            i + 1,
                            e
                        ))
                    })?;
                    let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    let text = v.get("content").and_then(|c| c.as_str()).unwrap_or("").trim();
                    if text.is_empty() {
                        continue;
                    }
                    let speaker = match role {
                        "user" | "human" => "user",
                        "assistant" | "ai" | "bot" => "assistant",
                        _ => {
                            return Err(MemoryError::BadRequest(format!(
                                "第 {} 行 role 必须是 user/assistant（含 human/ai 别名），实得 {:?}",
                                i + 1,
                                role
                            )))
                        }
                    };
                    out.push(serde_json::json!({"speaker": speaker, "text": text}));
                }
                out
            }
            "text" => content
                .split("\n\n")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .enumerate()
                .map(|(i, seg)| {
                    serde_json::json!({"speaker": if i % 2 == 0 { "user" } else { "assistant" }, "text": seg})
                })
                .collect(),
            _ => {
                return Err(MemoryError::BadRequest(format!(
                    "未知导入格式 {:?}：支持 jsonl / text",
                    format
                )))
            }
        };
        if turns.is_empty() {
            return Err(MemoryError::BadRequest(
                "导入内容为空——没有任何有效轮次".into(),
            ));
        }
        Ok(serde_json::Value::Array(turns))
    }

    /// 增量追加轮次到既有会话（长对话分片落库，不等收尾——自动节律 b 配套）。
    /// 只允许追加未蒸馏（pending）会话：已蒸馏的会话追加会割裂 L1 溯源。
    /// agent 可选补记（首个 append 补上会话归属，多 agent 视角的数据从现在记对）。
    pub async fn append_session(
        &self,
        id: Uuid,
        turns: serde_json::Value,
        agent: Option<&str>,
        distill: &str,
    ) -> Result<SessionDto, MemoryError> {
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("追加至少一轮".into()));
        }
        validate_turns(arr)?;
        let cur = sqlx::query_as::<_, SessionDto>("SELECT * FROM raw_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))?;
        if cur.distill_status != "pending" {
            return Err(MemoryError::BadRequest(format!(
                "会话已蒸馏（{}），不可追加——请开新会话",
                cur.distill_status
            )));
        }
        // P12：原子 jsonb 数组拼接——单语句在行级天然串行，并发 append 不丢更新
        // （分立的读-改-写在池连接上无法持锁，后写会覆盖前写的合并结果）。
        let row = sqlx::query_as::<_, SessionDto>(
            "UPDATE raw_sessions SET content = content || $2, agent = COALESCE($3, agent) WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(sqlx::types::Json(&serde_json::Value::Array(arr.to_vec())))
        .bind(agent)
        .fetch_one(&self.pool)
        .await?;
        // 追加同样走防抖：同窗口的追加与首写共用一个 extract 任务
        if distill == "auto" {
            engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                .await
                .ok();
        }
        Ok(row)
    }

    pub async fn list_sessions(
        &self,
        agent: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<SessionDto>, MemoryError> {
        Ok(sqlx::query_as::<_, SessionDto>(
            "SELECT * FROM raw_sessions \
             WHERE ($1::text IS NULL OR agent = $1) AND ($2::timestamptz IS NULL OR created_at < $2) \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(agent)
        .bind(cursor)
        .bind(limit.min(200))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        sqlx::query_as::<_, SessionDto>("SELECT * FROM raw_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))
    }

    /// L0 擦除：删会话 + 引用它的 atoms 标记来源失效。
    pub async fn erase_session(&self, id: Uuid) -> Result<(), MemoryError> {
        let affected = sqlx::query("DELETE FROM raw_sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(MemoryError::NotFound(format!("会话 {id} 不存在")));
        }
        // 来源失效标记：source_refs 里含该会话的原子加 erased 标记
        let atoms: Vec<(Uuid, serde_json::Value)> =
            sqlx::query_as("SELECT id, source_refs FROM atoms WHERE source_refs::text LIKE $1")
                .bind(format!("%{id}%"))
                .fetch_all(&self.pool)
                .await?;
        for (aid, refs) in atoms {
            let marked = mark_erased(refs, id);
            sqlx::query("UPDATE atoms SET source_refs = $2, updated_at = now() WHERE id = $1")
                .bind(aid)
                .bind(sqlx::types::Json(&marked))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn trigger_distill(
        &self,
        full: bool,
        via: &str,
        by: &str,
    ) -> Result<Vec<Job>, MemoryError> {
        engram_distill::chain::trigger(&self.queue, full, via, by)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))
    }

    /// 节律状态（memory-rhythm）：外部 cron 的心跳与积压年龄，供设置页判定逾期。
    /// last_heartbeat 复用 jobs 审计行（kind=rhythm_heartbeat）；pending 统计扫
    /// raw_sessions 积压（cron 兜底蒸馏的对象）。
    pub async fn rhythm_status(&self) -> Result<serde_json::Value, MemoryError> {
        let heartbeat: Option<(chrono::DateTime<chrono::Utc>, String)> = sqlx::query_as(
            "SELECT created_at, payload->>'by' AS by FROM jobs \
             WHERE kind = 'rhythm_heartbeat' ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let pending: (i64, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
            "SELECT count(*), min(created_at) FROM raw_sessions WHERE distill_status = 'pending'",
        )
        .fetch_one(&self.pool)
        .await?;
        let (count, oldest) = pending;
        let age_secs = oldest.map(|t| (chrono::Utc::now() - t).num_seconds());
        Ok(serde_json::json!({
            "last_heartbeat": heartbeat.as_ref().map(|h| h.0),
            "last_heartbeat_by": heartbeat.as_ref().map(|h| h.1.clone()),
            "pending_sessions": count,
            "oldest_pending_age_secs": age_secs,
        }))
    }

    // ---------- L1 ----------

    pub async fn list_atoms(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
        needs_review: Option<bool>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<AtomDto>, MemoryError> {
        Ok(sqlx::query_as::<_, AtomDto>(
            "SELECT * FROM atoms \
             WHERE ($1::text IS NULL OR kind = $1) AND ($2::text IS NULL OR status = $2) \
               AND ($3::bool IS NULL OR needs_review = $3) \
               AND ($4::timestamptz IS NULL OR created_at < $4) \
             ORDER BY created_at DESC LIMIT $5",
        )
        .bind(kind)
        .bind(status)
        .bind(needs_review)
        .bind(cursor)
        .bind(limit.min(500))
        .fetch_all(&self.pool)
        .await?)
    }

    /// 手工新增（人审补充；active 直接入库）。
    pub async fn create_atom(
        &self,
        kind: &str,
        content: &str,
        confidence: f32,
        occurred_at: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
        sensitive: bool,
    ) -> Result<AtomDto, MemoryError> {
        let text = content.trim();
        // 输入校验：空内容 + 超长（对齐蒸馏链的 1~120 字契约）
        if text.is_empty() {
            return Err(MemoryError::BadRequest("原子内容不能为空".into()));
        }
        if text.chars().count() > 120 {
            return Err(MemoryError::BadRequest(format!(
                "原子内容超长：最多 120 字，当前 {} 字",
                text.chars().count()
            )));
        }
        // A4 幂等护栏：同 kind + 同内容（trim 后）的 active 原子已存在则直接返回它——
        // AI 重试/重复直写不会双份（2026-08-31 测试方实测两条一模一样的生日原子）。
        // 近重复的语义合并仍归 arbitrate/consolidate，这里只挡精确重复。
        if let Some(existing) = sqlx::query_as::<_, AtomDto>(
            "SELECT * FROM atoms WHERE kind = $1 AND content = $2 AND status = 'active' LIMIT 1",
        )
        .bind(kind)
        .bind(text)
        .fetch_optional(&self.pool)
        .await?
        {
            tracing::info!(atom_id = %existing.id, "直写命中已有同内容原子，幂等返回");
            return Ok(existing);
        }
        // A1：与蒸馏链同规则——置信 <0.55 自动进人审，不直接生效污染记忆库
        // （此前直写硬编码 needs_review=false，文档/CLI 提示/实现三方打架）。
        let needs_review = confidence < 0.55;
        let id = Uuid::now_v7();
        let emb = self.try_embed(&[text.to_string()]).await;
        let row = sqlx::query_as::<_, AtomDto>(
            "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, sensitive, occurred_at, valid_until, source_refs, embedding, tsv) \
             VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8, '[]'::jsonb, $9, to_tsvector('simple', $10)) RETURNING *",
        )
        .bind(id)
        .bind(kind)
        .bind(text)
        .bind(confidence)
        .bind(needs_review)
        .bind(sensitive)
        .bind(occurred_at)
        .bind(valid_until)
        .bind(emb.as_ref().and_then(|v| v.first()).map(|v| pgvector::Vector::from(v.clone())))
        .bind(engram_search::tokenize::tsv_text(text))
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    // 选项袋式更新：8 个可选字段一一对应列；struct 化留给下一轮接口收敛
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub async fn update_atom(
        &self,
        id: Uuid,
        content: Option<&str>,
        kind: Option<&str>,
        confidence: Option<f32>,
        status: Option<&str>,
        needs_review: Option<bool>,
        superseded_by: Option<Uuid>,
        occurred_at: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
        sensitive: Option<bool>,
        // 编辑来源（"admin" / "key:名"）——改写语义时落 atom_revisions + 审计
        actor: &str,
    ) -> Result<AtomDto, MemoryError> {
        let cur = sqlx::query_as::<_, AtomDto>("SELECT * FROM atoms WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("原子 {id} 不存在")))?;

        let new_content = content.unwrap_or(&cur.content).to_string();
        let new_conf = confidence.unwrap_or(cur.confidence);
        let new_status = match status {
            Some("archived") => "archived",
            Some("active") if cur.status == "archived" => "active",
            Some("active") | Some("superseded") | Some("candidate") => {
                return Err(MemoryError::BadRequest(
                    "status 只允许 active/archived 切换；supersede 走矛盾流程".into(),
                ));
            }
            _ => cur.status.as_str(),
        };
        let content_changed = new_content != cur.content;
        let new_kind = kind.unwrap_or(&cur.kind);
        // 编辑能力：改写语义（content/kind/confidence）变化 → 旧值进 atom_revisions + 审计行。
        // AI 走 correction（新原子+superseded_by）不产生 revision；这条是用户轻量修正路。
        let rewrite = content_changed
            || new_kind != cur.kind
            || confidence.is_some_and(|c| (c - cur.confidence).abs() > f32::EPSILON);
        if rewrite {
            sqlx::query(
                "INSERT INTO atom_revisions (id, atom_id, old_content, old_kind, old_confidence, edited_by) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(Uuid::now_v7())
            .bind(id)
            .bind(&cur.content)
            .bind(&cur.kind)
            .bind(cur.confidence)
            .bind(actor)
            .execute(&self.pool)
            .await?;
            self.audit(
                "edit_atom",
                json!({
                    "atom_id": id.to_string(),
                    "by": actor,
                    "old": {"content": cur.content, "kind": cur.kind, "confidence": cur.confidence},
                    "new": {
                        "content": if content_changed { new_content.clone() } else { cur.content.clone() },
                        "kind": new_kind,
                        "confidence": confidence.unwrap_or(cur.confidence),
                    },
                }),
            )
            .await;
        }
        let emb = if content_changed {
            self.try_embed(std::slice::from_ref(&new_content)).await
        } else {
            None
        };

        let row = sqlx::query_as::<_, AtomDto>(
            "UPDATE atoms SET content = $2, confidence = $3, status = $4, kind = $12, needs_review = COALESCE($5, needs_review), \
                 superseded_by = COALESCE($6, superseded_by), occurred_at = COALESCE($7, occurred_at), \
                 valid_until = COALESCE($8, valid_until), sensitive = COALESCE($9, sensitive), \
                 embedding = COALESCE($10, embedding), tsv = to_tsvector('simple', $11), updated_at = now() \
             WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(&new_content)
        .bind(new_conf)
        .bind(new_status)
        .bind(needs_review)
        .bind(superseded_by)
        .bind(occurred_at)
        .bind(valid_until)
        .bind(sensitive)
        .bind(emb.as_ref().and_then(|v| v.first()).map(|v| pgvector::Vector::from(v.clone())))
        .bind(engram_search::tokenize::tsv_text(&new_content))
        .bind(new_kind)
        .fetch_one(&self.pool)
        .await?;

        // F4 治：归档或标敏感 → 受影响场景快照需要收敛重算（best-effort 异步，
        // 30s 防抖合并批量归档；重算仅活跃非敏感成员、0 活跃则解散——organize 收敛段）
        if new_status == "archived" || sensitive == Some(true) {
            let bucket = chrono::Utc::now().timestamp() / self.debounce_secs;
            self.queue
                .enqueue(
                    JobTemplate::new("organize_scenarios")
                        .with_idempotency_key(format!("snapshot-refresh-{bucket}"))
                        .with_payload(
                            serde_json::json!({"converge_only": true, "atom_id": id.to_string()}),
                        )
                        .with_due(
                            chrono::Utc::now() + chrono::Duration::seconds(self.debounce_secs),
                        ),
                )
                .await
                .ok();
        }
        Ok(row)
    }

    // ---------- L2 / L3 ----------

    pub async fn list_scenarios(&self, limit: i64) -> Result<Vec<ScenarioDto>, MemoryError> {
        Ok(sqlx::query_as::<_, ScenarioDto>(
            "SELECT * FROM scenarios ORDER BY updated_at DESC LIMIT $1",
        )
        .bind(limit.min(200))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_scenario(&self, id: Uuid) -> Result<ScenarioDto, MemoryError> {
        sqlx::query_as::<_, ScenarioDto>("SELECT * FROM scenarios WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("场景 {id} 不存在")))
    }

    /// 当前画像（每分面最新版）。
    pub async fn persona(&self) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(sqlx::query_as::<_, PersonaVersion>(
            "SELECT DISTINCT ON (aspect) * FROM persona_aspects ORDER BY aspect, version DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// 分面版本历史。
    pub async fn persona_history(&self, aspect: &str) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(sqlx::query_as::<_, PersonaVersion>(
            "SELECT * FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC",
        )
        .bind(aspect)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 回滚分面到历史版本（以新版本号落地当前内容——历史不可变）。
    /// 编辑能力：用户直接改画像分面（仅用户会话；AI 禁入）。钉住 = 蒸馏绕开。
    pub async fn persona_edit(
        &self,
        aspect: &str,
        content: &str,
        actor: &str,
    ) -> Result<PersonaVersion, MemoryError> {
        let content = content.trim();
        if content.is_empty() || content.chars().count() > 4000 {
            return Err(MemoryError::BadRequest("分面内容需 1~4000 字".into()));
        }
        let cur: Option<Option<i32>> =
            sqlx::query_scalar("SELECT MAX(version) FROM persona_aspects WHERE aspect = $1")
                .bind(aspect)
                .fetch_optional(&self.pool)
                .await?;
        let next_v = cur.flatten().map(|v| v + 1).unwrap_or(1);
        sqlx::query(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, manually_edited) \
             VALUES ($1, $2, $3, '[]'::jsonb, $4, 'human', true)",
        )
        .bind(Uuid::now_v7())
        .bind(aspect)
        .bind(content)
        .bind(next_v)
        .execute(&self.pool)
        .await?;
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "edit", "version": next_v,
            }),
        )
        .await;
        self.persona_history(aspect)
            .await
            .map(|mut v| v.swap_remove(0))
    }

    /// 解除钉住：分面回归蒸馏管辖（下次 consolidate/退休可重写）。
    pub async fn persona_unpin(&self, aspect: &str, actor: &str) -> Result<(), MemoryError> {
        sqlx::query(
            "UPDATE persona_aspects SET manually_edited = false \
             WHERE id IN (SELECT id FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC LIMIT 1)",
        )
        .bind(aspect)
        .execute(&self.pool)
        .await?;
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "unpin",
            }),
        )
        .await;
        Ok(())
    }

    /// 原子改写历史（新→旧）。
    pub async fn atom_revisions(&self, atom_id: Uuid) -> Result<Vec<AtomRevision>, MemoryError> {
        Ok(sqlx::query_as::<_, AtomRevision>(
            "SELECT * FROM atom_revisions WHERE atom_id = $1 ORDER BY created_at DESC",
        )
        .bind(atom_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 实体摘要版本链（圈子强化）：手编档案的历史，最近在前。
    pub async fn entity_revisions(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<EntityRevision>, MemoryError> {
        Ok(sqlx::query_as::<_, EntityRevision>(
            "SELECT * FROM entity_revisions WHERE entity_id = $1 ORDER BY created_at DESC",
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 实体关系列表（有向类型化；可选按实体过滤 from/to 两端）。
    pub async fn list_relations(
        &self,
        entity_id: Option<Uuid>,
    ) -> Result<Vec<EntityRelationDto>, MemoryError> {
        let rows = match entity_id {
            Some(eid) => {
                sqlx::query_as::<_, EntityRelationDto>(
                    "SELECT * FROM entity_relations WHERE from_id = $1 OR to_id = $1 ORDER BY created_at DESC",
                )
                .bind(eid)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, EntityRelationDto>("SELECT * FROM entity_relations ORDER BY created_at DESC")
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        Ok(rows)
    }

    /// 建关系（有向）：from --rel_type--> to；同向同类型 upsert（weight 累加）。
    pub async fn create_relation(
        &self,
        from: Uuid,
        to: Uuid,
        rel_type: &str,
        source: &str,
    ) -> Result<EntityRelationDto, MemoryError> {
        if from == to {
            return Err(MemoryError::BadRequest("关系两端不能是同一实体".into()));
        }
        if !REL_TYPES.contains(&rel_type) {
            return Err(MemoryError::BadRequest(format!(
                "rel_type 只允许 {}",
                REL_TYPES.join("/")
            )));
        }
        self.entity_row(from).await?;
        self.entity_row(to).await?;
        let row = sqlx::query_as::<_, EntityRelationDto>(
            "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) \
             VALUES ($1, $2, $3, $4, 1, $5) \
             ON CONFLICT (from_id, to_id, rel_type) DO UPDATE SET weight = entity_relations.weight + 1, updated_at = now() \
             RETURNING *",
        )
        .bind(Uuid::now_v7())
        .bind(from)
        .bind(to)
        .bind(rel_type)
        .bind(source)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// 删关系。
    pub async fn delete_relation(&self, id: Uuid) -> Result<(), MemoryError> {
        let n = sqlx::query("DELETE FROM entity_relations WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(MemoryError::NotFound(format!("关系 {id} 不存在")));
        }
        Ok(())
    }

    /// 重新钉住（用户解锁后想再钉：为当前内容写一条钉住版本）。
    pub async fn persona_repin(
        &self,
        aspect: &str,
        actor: &str,
    ) -> Result<PersonaVersion, MemoryError> {
        let cur: Option<(String, i32)> = sqlx::query_as(
            "SELECT content, version FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC LIMIT 1",
        )
        .bind(aspect)
        .fetch_optional(&self.pool)
        .await?;
        let Some((content, v)) = cur else {
            return Err(MemoryError::NotFound(format!("分面 {aspect} 不存在")));
        };
        sqlx::query(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, manually_edited) \
             VALUES ($1, $2, $3, '[]'::jsonb, $4, 'human', true)",
        )
        .bind(Uuid::now_v7())
        .bind(aspect)
        .bind(&content)
        .bind(v + 1)
        .execute(&self.pool)
        .await?;
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "repin", "version": v + 1,
            }),
        )
        .await;
        self.persona_history(aspect)
            .await
            .map(|mut h| h.swap_remove(0))
    }

    pub async fn persona_rollback(
        &self,
        aspect: &str,
        to_version: i32,
        actor: &str,
    ) -> Result<PersonaVersion, MemoryError> {
        let target = sqlx::query_as::<_, PersonaVersion>(
            "SELECT * FROM persona_aspects WHERE aspect = $1 AND version = $2",
        )
        .bind(aspect)
        .bind(to_version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| MemoryError::NotFound(format!("版本 {aspect}#{to_version} 不存在")))?;

        let cur: Option<Option<i32>> =
            sqlx::query_scalar("SELECT MAX(version) FROM persona_aspects WHERE aspect = $1")
                .bind(aspect)
                .fetch_optional(&self.pool)
                .await?;
        let next_v = cur.flatten().map(|v| v + 1).unwrap_or(1);

        sqlx::query(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, manually_edited) \
             VALUES ($1, $2, $3, $4::jsonb, $5, 'rollback', true)",
        )
        .bind(Uuid::now_v7())
        .bind(aspect)
        .bind(&target.content)
        .bind(sqlx::types::Json(&json!({"rollback_to": to_version})))
        .bind(next_v)
        .execute(&self.pool)
        .await?;
        // 回滚 = 人工钉住（蒸馏绕开，直到解锁）
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "rollback", "to_version": to_version,
            }),
        )
        .await;
        self.persona_history(aspect)
            .await
            .map(|mut v| v.swap_remove(0))
    }

    // ---------- 实体（记忆星系） ----------

    /// 活体实体列表（按记忆密度降序）。
    pub async fn list_entities(&self, kind: Option<&str>) -> Result<Vec<EntityDto>, MemoryError> {
        Ok(sqlx::query_as::<_, EntityDto>(
            "SELECT e.id, e.name, e.kind, e.summary, count(ae.atom_id)::bigint AS atom_count, e.manually_edited, e.updated_at \
             FROM entities e LEFT JOIN atom_entities ae ON ae.entity_id = e.id \
             WHERE e.merged_into IS NULL AND ($1::text IS NULL OR e.kind = $1) \
             GROUP BY e.id, e.name, e.kind, e.summary, e.manually_edited, e.updated_at \
             ORDER BY atom_count DESC, e.updated_at DESC",
        )
        .bind(kind)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn entity_row(&self, id: Uuid) -> Result<EntityDto, MemoryError> {
        sqlx::query_as::<_, EntityDto>(
            "SELECT e.id, e.name, e.kind, e.summary, count(ae.atom_id)::bigint AS atom_count, bool_or(e.manually_edited) AS manually_edited, e.updated_at \
             FROM entities e LEFT JOIN atom_entities ae ON ae.entity_id = e.id \
             WHERE e.id = $1 AND e.merged_into IS NULL \
             GROUP BY e.id, e.name, e.kind, e.summary, e.manually_edited, e.updated_at",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| MemoryError::NotFound(format!("实体 {id} 不存在")))
    }

    /// 实体详情：画像摘要 + 相关原子时间线 + 相关场景。
    pub async fn get_entity(&self, id: Uuid) -> Result<EntityDetail, MemoryError> {
        let entity = self.entity_row(id).await?;
        let atoms = sqlx::query_as::<_, AtomDto>(
            "SELECT a.* FROM atoms a JOIN atom_entities ae ON ae.atom_id = a.id \
             WHERE ae.entity_id = $1 ORDER BY a.created_at DESC LIMIT 200",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let scenarios = sqlx::query_as::<_, ScenarioDto>(
            "SELECT DISTINCT ON (s.id) s.* FROM scenarios s \
             JOIN atoms a ON a.scenario_id = s.id \
             JOIN atom_entities ae ON ae.atom_id = a.id \
             WHERE ae.entity_id = $1 LIMIT 50",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        // 共现邻居：与当前实体共享原子的其他实体，按共现次数降序
        let neighbors = sqlx::query_as::<_, EntityDto>(
            "SELECT e.id, e.name, e.kind, e.summary, \
                    (SELECT count(*) FROM atom_entities x WHERE x.entity_id = e.id)::bigint AS atom_count, \
                    bool_or(e.manually_edited) AS manually_edited, e.updated_at \
             FROM atom_entities ae \
             JOIN entities e ON e.id = ae.entity_id \
             WHERE ae.atom_id IN (SELECT atom_id FROM atom_entities WHERE entity_id = $1) \
               AND ae.entity_id != $1 AND e.merged_into IS NULL \
             GROUP BY e.id, e.name, e.kind, e.summary, e.manually_edited, e.updated_at \
             ORDER BY count(*) DESC, e.updated_at DESC LIMIT 20",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        // 类型化关系：本实体作为 from 或 to 的有向关系
        let relations = sqlx::query_as::<_, EntityRelationDto>(
            "SELECT * FROM entity_relations WHERE from_id = $1 OR to_id = $1 ORDER BY created_at DESC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        Ok(EntityDetail {
            entity,
            atoms,
            scenarios,
            neighbors,
            relations,
        })
    }

    /// 手动建实体（蒸馏自动抽取之外的人工入口；同名同类活体只许一个）。
    pub async fn create_entity(
        &self,
        name: &str,
        kind: &str,
        summary: &str,
    ) -> Result<EntityDto, MemoryError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(MemoryError::BadRequest("实体名需 1~60 字".into()));
        }
        if !ENTITY_KINDS.contains(&kind) {
            return Err(MemoryError::BadRequest(format!(
                "kind 只允许 {}",
                ENTITY_KINDS.join("/")
            )));
        }
        let dup: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM entities WHERE name = $1 AND kind = $2 AND merged_into IS NULL",
        )
        .bind(name)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await?;
        if dup.is_some() {
            return Err(MemoryError::BadRequest(format!(
                "同名同类实体已存在：{name}"
            )));
        }
        sqlx::query("INSERT INTO entities (id, name, kind, summary) VALUES ($1, $2, $3, $4)")
            .bind(Uuid::now_v7())
            .bind(name)
            .bind(kind)
            .bind(summary)
            .execute(&self.pool)
            .await?;
        self.entity_row(
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM entities WHERE name = $1 AND kind = $2 AND merged_into IS NULL",
            )
            .bind(name)
            .bind(kind)
            .fetch_one(&self.pool)
            .await?,
        )
        .await
    }

    pub async fn update_entity(
        &self,
        id: Uuid,
        name: Option<&str>,
        summary: Option<&str>,
        actor: &str,
    ) -> Result<EntityDto, MemoryError> {
        let mut changed = false;
        if let Some(n) = name {
            let n = n.trim();
            if n.is_empty() || n.chars().count() > 60 {
                return Err(MemoryError::BadRequest("实体名需 1~60 字".into()));
            }
            sqlx::query("UPDATE entities SET name = $2, updated_at = now() WHERE id = $1 AND merged_into IS NULL")
                .bind(id)
                .bind(n)
                .execute(&self.pool)
                .await?;
            changed = true;
        }
        if let Some(s) = summary {
            // 版本链：旧摘要进 entity_revisions（轻量历史，复用原子 revisions 模式）
            let old: Option<String> = sqlx::query_scalar(
                "SELECT summary FROM entities WHERE id = $1 AND merged_into IS NULL",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(old_summary) = old
                && old_summary != s
            {
                sqlx::query(
                    "INSERT INTO entity_revisions (id, entity_id, old_summary, edited_by) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(Uuid::now_v7())
                .bind(id)
                .bind(&old_summary)
                .bind(actor)
                .execute(&self.pool)
                .await?;
            }
            sqlx::query(
                "UPDATE entities SET summary = $2, manually_edited = true, updated_at = now() \
                         WHERE id = $1 AND merged_into IS NULL",
            )
            .bind(id)
            .bind(s)
            .execute(&self.pool)
            .await?;
            changed = true;
        }
        if changed {
            // 用户手编实体档案 → 钉住（consolidate 档案重生成绕开）；审计
            sqlx::query(
                "UPDATE entities SET manually_edited = true WHERE id = $1 AND merged_into IS NULL",
            )
            .bind(id)
            .execute(&self.pool)
            .await?;
            self.audit(
                "edit_entity",
                json!({
                    "entity_id": id.to_string(), "by": actor,
                }),
            )
            .await;
        }
        self.entity_row(id).await
    }

    pub async fn delete_entity(&self, id: Uuid) -> Result<(), MemoryError> {
        // 只有活体可删；删活体时连带清掉并入它的墓碑（merged_into 指向它）——
        // 否则 FK(entities_merged_into_fkey) 拒绝删除。墓碑是合并的残迹，赢家没了它也不复活。
        let n = sqlx::query(
            "DELETE FROM entities WHERE ($1 IN (SELECT id FROM entities WHERE id = $1 AND merged_into IS NULL)) \
             AND (id = $1 OR merged_into = $1)",
        )
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(MemoryError::NotFound(format!("实体 {id} 不存在")));
        }
        // 审计（2026-09-01 补：entity 删除曾无审计行，排查全靠猜）——best-effort
        self.audit(
            "delete_entity",
            serde_json::json!({ "entity_id": id, "tombstones": n - 1 }),
        )
        .await;
        Ok(())
    }

    /// 实体级遗忘（「把小王忘了」）：级联归档挂链 active 原子 → 摘链 → 删实体+墓碑。
    /// archived/superseded 等非 active 原子不动（本来就是历史）；人审候选一并归档。
    pub async fn forget_entity(&self, id: Uuid) -> Result<usize, MemoryError> {
        let cur = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM entities WHERE id = $1 AND merged_into IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        if cur.is_none() {
            return Err(MemoryError::NotFound(format!("实体 {id} 不存在或已合并")));
        }
        let n = sqlx::query(
            "UPDATE atoms SET status = 'archived', updated_at = now() \
             WHERE status = 'active' AND id IN (SELECT atom_id FROM atom_entities WHERE entity_id = $1)",
        )
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected() as usize;
        // 摘链（atom_entities 随实体删除本会级联，这里显式删保持语义清晰）
        sqlx::query("DELETE FROM atom_entities WHERE entity_id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.delete_entity(id).await?;
        Ok(n)
    }

    /// P5 会话作废：「这段白记了」——标记 void，蒸馏跳过（claim 只取 pending）。
    /// 只允许 pending 会话作废（已蒸馏的产出用 purge 清场处理）。
    /// M-2（2026-09-03）：「不存在」404 与「非 pending」400 分开报，不再合并一句。
    pub async fn void_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        let status: Option<String> =
            sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        match status.as_deref() {
            None => return Err(MemoryError::NotFound(format!("会话 {id} 不存在"))),
            Some("pending") => {}
            Some(s) => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 已处理（当前状态 {s}）——作废只对未蒸馏会话；已蒸馏的用 purge 清场"
                )));
            }
        }
        let row = sqlx::query_as::<_, SessionDto>(
            "UPDATE raw_sessions SET distill_status = 'void' WHERE id = $1 AND distill_status = 'pending' RETURNING *",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| {
            // 查询与更新之间的竞态兜底（状态刚被蒸馏 worker 抢走）
            MemoryError::BadRequest(format!(
                "会话 {id} 刚被蒸馏任务取走（processing）——请稍后用 purge 清场"
            ))
        })
    }

    /// P11/SEC-E 按 agent 清场（测试隔离，2026-09-03 彻底化）：该 agent **全部**会话
    /// 物理删除（pending/processing/done/void 一视同仁，sensitive 原文不留——
    /// 此前 done 会话残留曾导致敏感原始对话留库）+ 其产出的 active 原子归档（可恢复）。
    /// 返回 (erased_sessions, archived_atoms)。顺序敏感：先归档原子（JOIN 会话判归属）
    /// 再删会话——删会话后 JOIN 不可判归属。
    pub async fn purge_agent(&self, agent: &str) -> Result<(i64, i64), MemoryError> {
        let mut tx = self.pool.begin().await.map_err(MemoryError::from)?;
        let archived = sqlx::query(
            "UPDATE atoms SET status = 'archived', updated_at = now() \
             WHERE status = 'active' AND EXISTS ( \
                SELECT 1 FROM jsonb_array_elements(atoms.source_refs) e \
                JOIN raw_sessions s ON s.id::text = e->>'session_id' \
                WHERE s.agent = $1)",
        )
        .bind(agent)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let erased = sqlx::query("DELETE FROM raw_sessions WHERE agent = $1")
            .bind(agent)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await.map_err(MemoryError::from)?;
        Ok((erased as i64, archived as i64))
    }

    /// F1/F2 deep purge（终极清空测试）：记忆域四层 + 实体链一键清空，单事务，
    /// 返回五计数。TRUNCATE CASCADE 一发解 FK——API 层负责 erase scope + confirm 双因子。
    pub async fn purge_deep(&self) -> Result<serde_json::Value, MemoryError> {
        purge_deep_pool(&self.pool).await.map_err(MemoryError::from)
    }

    /// P-C 阶段一：arm——入队 5 分钟冷却的 deep_purge job（后悔药窗口）。
    pub async fn arm_deep_purge(&self, source: &str) -> Result<Job, MemoryError> {
        // 秒级防抖：同秒连点只 arm 一次；跨秒可重新 arm（取消后立刻重 arm 是正当操作）
        let bucket = chrono::Utc::now().timestamp();
        self.queue
            .enqueue(
                JobTemplate::new("deep_purge")
                    .with_idempotency_key(format!("deep-purge-arm-{bucket}"))
                    .with_payload(json!({
                        "phase": "armed",
                        "confirm": PURGE_CONFIRM_PHRASE,
                        "authorized_by": source,
                    }))
                    .with_due(chrono::Utc::now() + chrono::Duration::minutes(5)),
            )
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))
    }

    /// 编辑/清空类审计：写一条已完成的 job 行（谁、何时、干了什么）——不可抵赖凭证。
    pub async fn audit(&self, kind: &str, payload: serde_json::Value) {
        sqlx::query(
            "INSERT INTO jobs (id, kind, payload, status, attempts, max_attempts, \
             progress, started_at, finished_at) \
             VALUES ($1, $2, $3, 'succeeded', 1, 1, $3, now(), now())",
        )
        .bind(Uuid::now_v7())
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await
        .ok();
    }

    /// P4 全量导出（数据主权）：记忆域五表完整快照，JSON 随身带走。
    /// R4：sensitive 原子默认排除（隐私面不随导出扩大到文件系统），
    /// include_sensitive=true 显式包含——与检索 reveal 同权。
    pub async fn export(&self, include_sensitive: bool) -> Result<serde_json::Value, MemoryError> {
        let sessions: Vec<SessionDto> =
            sqlx::query_as("SELECT * FROM raw_sessions ORDER BY created_at")
                .fetch_all(&self.pool)
                .await?;
        let atoms: Vec<AtomDto> = sqlx::query_as(if include_sensitive {
            "SELECT * FROM atoms ORDER BY created_at"
        } else {
            "SELECT * FROM atoms WHERE NOT sensitive ORDER BY created_at"
        })
        .fetch_all(&self.pool)
        .await?;
        let scenarios: Vec<ScenarioDto> =
            sqlx::query_as("SELECT * FROM scenarios ORDER BY created_at")
                .fetch_all(&self.pool)
                .await?;
        let persona: Vec<PersonaVersion> =
            sqlx::query_as("SELECT * FROM persona_aspects ORDER BY aspect, version")
                .fetch_all(&self.pool)
                .await?;
        let entities: Vec<EntityDto> =
            sqlx::query_as(
                "SELECT id, name, kind, summary, \
                 (SELECT count(*) FROM atom_entities ae WHERE ae.entity_id = entities.id) AS atom_count, \
                 manually_edited, updated_at FROM entities WHERE merged_into IS NULL ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool).await?;
        Ok(serde_json::json!({
            "format": "engram-memory-export",
            "version": 1,
            "exported_at": chrono::Utc::now(),
            "counts": {
                "sessions": sessions.len(), "atoms": atoms.len(),
                "scenarios": scenarios.len(), "persona": persona.len(),
                "entities": entities.len(),
            },
            "sensitive_excluded": !include_sensitive,
            "sessions": sessions, "atoms": atoms, "scenarios": scenarios,
            "persona": persona, "entities": entities,
        }))
    }

    /// 挂原子到实体（幂等）。
    pub async fn attach_atom(&self, entity_id: Uuid, atom_id: Uuid) -> Result<(), MemoryError> {
        self.entity_row(entity_id).await?;
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE id = $1")
            .bind(atom_id)
            .fetch_one(&self.pool)
            .await?;
        if n == 0 {
            return Err(MemoryError::NotFound(format!("原子 {atom_id} 不存在")));
        }
        sqlx::query(
            "INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2) \
             ON CONFLICT DO NOTHING",
        )
        .bind(atom_id)
        .bind(entity_id)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE entities SET updated_at = now() WHERE id = $1")
            .bind(entity_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn detach_atom(&self, entity_id: Uuid, atom_id: Uuid) -> Result<(), MemoryError> {
        let n = sqlx::query("DELETE FROM atom_entities WHERE atom_id = $1 AND entity_id = $2")
            .bind(atom_id)
            .bind(entity_id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(MemoryError::NotFound(format!(
                "原子 {atom_id} 未关联到实体 {entity_id}"
            )));
        }
        Ok(())
    }

    /// 合并实体：from 的原子关联全部改挂 into，from 置 merged_into 让出唯一名。
    pub async fn merge_entities(&self, from: Uuid, into: Uuid) -> Result<i64, MemoryError> {
        if from == into {
            return Err(MemoryError::BadRequest("不能合并到自身".into()));
        }
        self.entity_row(from).await?;
        self.entity_row(into).await?;
        let moved = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO atom_entities (atom_id, entity_id) \
             SELECT atom_id, $2 FROM atom_entities WHERE entity_id = $1 \
             ON CONFLICT DO NOTHING RETURNING atom_id",
        )
        .bind(from)
        .bind(into)
        .fetch_all(&self.pool)
        .await?
        .len() as i64;
        sqlx::query("DELETE FROM atom_entities WHERE entity_id = $1")
            .bind(from)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE entities SET merged_into = $2, updated_at = now() WHERE id = $1")
            .bind(from)
            .bind(into)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE entities SET updated_at = now() WHERE id = $1")
            .bind(into)
            .execute(&self.pool)
            .await?;
        Ok(moved)
    }

    /// 星系图：节点（活体实体 + 密度）+ 共现边。
    pub async fn entity_graph(&self) -> Result<EntityGraph, MemoryError> {
        let nodes = self.list_entities(None).await?;
        let edges = sqlx::query_as::<_, GraphEdge>(
            "SELECT ae1.entity_id AS a, ae2.entity_id AS b, count(*)::bigint AS weight \
             FROM atom_entities ae1 \
             JOIN atom_entities ae2 ON ae1.atom_id = ae2.atom_id AND ae1.entity_id < ae2.entity_id \
             GROUP BY ae1.entity_id, ae2.entity_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let relations = self.list_relations(None).await?;
        Ok(EntityGraph {
            nodes,
            edges,
            relations,
        })
    }

    /// 全局记忆时间轴：原子（occurred_at 优先）/场景/实体按时间倒序合并。
    pub async fn timeline(&self, limit: i64) -> Result<Vec<TimelineEvent>, MemoryError> {
        Ok(sqlx::query_as::<_, TimelineEvent>(
            "SELECT a.id, COALESCE(a.occurred_at, a.created_at) AS at, 'atom' AS kind, a.content \
             FROM atoms a WHERE a.status = 'active' AND NOT a.sensitive \
             UNION ALL \
             SELECT s.id, s.created_at AS at, 'scenario' AS kind, s.topic \
             FROM scenarios s \
             UNION ALL \
             SELECT e.id, e.created_at AS at, 'entity' AS kind, e.name \
             FROM entities e WHERE e.merged_into IS NULL \
             ORDER BY at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 记忆域缺失向量统计（重嵌修复入口的状态面）。
    pub async fn embedding_status(&self) -> Result<EmbeddingStatus, MemoryError> {
        let atoms_missing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM atoms WHERE status = 'active' AND embedding IS NULL",
        )
        .fetch_one(&self.pool)
        .await?;
        let scenarios_missing: i64 =
            sqlx::query_scalar("SELECT count(*) FROM scenarios WHERE embedding IS NULL")
                .fetch_one(&self.pool)
                .await?;
        Ok(EmbeddingStatus {
            atoms_missing,
            scenarios_missing,
        })
    }

    /// 入队重嵌（换 embedding 供应商后的修复路径；job 见 distill::reembed）。
    pub async fn reembed(&self) -> Result<(), MemoryError> {
        self.queue
            .enqueue(
                JobTemplate::new("reembed_memory")
                    .with_idempotency_key(format!("reembed-memory-{}", Uuid::now_v7().simple())),
            )
            .await
            .map(|_| ())
            .map_err(|e| MemoryError::Storage(format!("入队失败: {e}")))
    }

    // ---------- 检索 ----------

    async fn try_embed(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        // L6：经记账门面（查询/场景嵌入计入用量）
        self.registry
            .embed_for(Purpose::Embed, texts.to_vec(), Some(1024), None)
            .await
            .ok()
            .map(|r| r.embeddings)
    }

    /// B9 命中反馈：检索命中即异步回写 hit_count（best-effort，失败只记日志）。
    /// 不刷 updated_at——hit 是使用热度而非内容变化，避免扰动「最近更新」排序。
    fn fire_hit_feedback(&self, table: &'static str, ids: Vec<Uuid>) {
        if ids.is_empty() {
            return;
        }
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let sql = if table == "atoms" {
                "UPDATE atoms SET hit_count = hit_count + 1 WHERE id = ANY($1)"
            } else {
                "UPDATE scenarios SET hit_count = hit_count + 1 WHERE id = ANY($1)"
            };
            if let Err(e) = sqlx::query(sql).bind(&ids).execute(&pool).await {
                tracing::warn!(error = %e, table, "hit_count 回写失败（不影响检索结果）");
            }
        });
    }

    /// 分层检索。无 embedding 通道时自动退化为纯 FTS。
    #[allow(clippy::too_many_arguments)]
    pub async fn search(
        &self,
        query: &str,
        layers: &[&str],
        max_items: i64,
        no_feedback: bool,
        reveal: bool,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<SearchResponse, MemoryError> {
        let qv = self
            .try_embed(&[query.to_string()])
            .await
            .and_then(|v| v.first().cloned());
        let all = layers.is_empty();
        let want_e = all || layers.contains(&"entities");
        let want_l1 = all || layers.contains(&"l1");
        let want_l2 = all || layers.contains(&"l2");
        let want_l3 = all || layers.contains(&"l3");

        // 实体：token 命中（名字加权）——主角先行
        let entities = if want_e {
            engram_search::search_entities(&self.pool, query, max_items).await?
        } else {
            vec![]
        };
        let l1 = if want_l1 {
            search_atoms(
                &self.pool,
                query,
                qv.as_deref(),
                max_items,
                reveal,
                from,
                to,
            )
            .await?
        } else {
            vec![]
        };
        let l2 = if want_l2 {
            search_scenarios(&self.pool, query, qv.as_deref(), max_items).await?
        } else {
            vec![]
        };
        // L3：小体量——jieba 分词双侧匹配打分排序（弃子串 contains：跨词边界/无序不可靠）
        let l3 = if want_l3 {
            let tokens: std::collections::HashSet<String> = tokenize(query).into_iter().collect();
            let mut scored: Vec<(usize, PersonaVersion)> = self
                .persona()
                .await?
                .into_iter()
                .map(|p| {
                    let ct: std::collections::HashSet<String> =
                        tokenize(&p.content).into_iter().collect();
                    let s = tokens.intersection(&ct).count();
                    (s, p)
                })
                .filter(|(s, _)| *s > 0)
                .collect();
            scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
            scored.into_iter().map(|(_, p)| p).collect()
        } else {
            vec![]
        };
        // B9：命中反馈（异步 best-effort，不阻塞返回）；no_feedback=true 跳过（B6 污染防护）
        if !no_feedback {
            self.fire_hit_feedback("atoms", l1.iter().map(|h| h.id).collect());
            self.fire_hit_feedback("scenarios", l2.iter().map(|h| h.id).collect());
        }
        Ok(SearchResponse {
            entities,
            l1,
            l2,
            l3,
            query: query.to_string(),
        })
    }

    /// 冷启动上下文包：L3 全量 + L2 相关/最近 + L1 补充，预算裁剪。
    pub async fn context_pack(
        &self,
        query: Option<&str>,
        budget_items: usize,
        budget_chars: usize,
        no_feedback: bool,
    ) -> Result<ContextPack, MemoryError> {
        let mut chars_used = 0usize;
        let mut truncated = false;

        let count = |s: &str, used: &mut usize, trunc: &mut bool| -> bool {
            if *used + s.len() > budget_chars {
                *trunc = true;
                false
            } else {
                *used += s.len();
                true
            }
        };

        // 有 query 时预计算 query 向量（L2/L1 共用，避免重复 embed）
        let qv: Option<Vec<f32>> = match query {
            Some(q) => self
                .try_embed(&[q.to_string()])
                .await
                .and_then(|v| v.first().cloned()),
            None => None,
        };

        // L3 全量（很小）
        let persona: Vec<PersonaVersion> = self
            .persona()
            .await?
            .into_iter()
            .take_while(|p| count(&p.content, &mut chars_used, &mut truncated))
            .collect();

        // L2：有 query 按相关性，否则最近
        let scenarios = if let Some(q) = query {
            search_scenarios(&self.pool, q, qv.as_deref(), (budget_items as i64).max(3)).await?
        } else {
            self.list_scenarios((budget_items as i64).max(3) / 2)
                .await?
                .into_iter()
                .map(|s| SearchHit {
                    id: s.id,
                    score: 0.0,
                    title: Some(s.topic.clone()),
                    snippet: s.summary.clone(),
                    kind: None,
                    needs_review: None,
                })
                .collect::<Vec<_>>()
        };
        let mut out_scenarios = Vec::new();
        for h in scenarios.into_iter().take(budget_items * 2 / 5) {
            match self.get_scenario(h.id).await {
                Ok(s)
                    if count(
                        &format!("{}{}", s.topic, s.summary),
                        &mut chars_used,
                        &mut truncated,
                    ) =>
                {
                    out_scenarios.push(s)
                }
                _ => break,
            }
        }

        // 实体透镜（小预算 ~20%）：AI 冷启动要知道用户世界里都有谁。
        // 有 query 走 token 相关（纯 jieba，不依赖向量）；无 query 按密度头部。best-effort。
        let ent_budget = (budget_items / 5).max(2);
        let entity_ids: Vec<Uuid> = match query {
            Some(q) => engram_search::search_entities(&self.pool, q, ent_budget as i64)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|h| h.id)
                .collect(),
            None => self
                .list_entities(None)
                .await
                .unwrap_or_default()
                .into_iter()
                .take(ent_budget)
                .map(|e| e.id)
                .collect(),
        };
        let mut out_entities = Vec::new();
        if !entity_ids.is_empty() {
            let rows: Vec<EntityDto> = sqlx::query_as(
                "SELECT id, name, kind, summary, \
                 (SELECT count(*) FROM atom_entities ae WHERE ae.entity_id = entities.id) AS atom_count, \
                 manually_edited, updated_at FROM entities WHERE id = ANY($1) AND merged_into IS NULL",
            )
                    .bind(&entity_ids)
                    .fetch_all(&self.pool)
                    .await?;
            // 保持命中序（query 路径相关性优先；无 query 路径密度优先）
            let by_id: std::collections::HashMap<Uuid, EntityDto> =
                rows.into_iter().map(|e| (e.id, e)).collect();
            for id in entity_ids {
                if let Some(e) = by_id.get(&id).cloned() {
                    if count(
                        &format!("{}{}", e.name, e.summary),
                        &mut chars_used,
                        &mut truncated,
                    ) {
                        out_entities.push(e);
                    } else {
                        truncated = true;
                        break;
                    }
                }
            }
        }

        // L1 补充（预算剩余）：有 query 按语义相关（search_atoms），否则 hit_count
        let remaining =
            budget_items.saturating_sub(persona.len() + out_scenarios.len() + out_entities.len());
        let atoms: Vec<AtomDto> = match query {
            Some(q) => {
                let hits = search_atoms(
                    &self.pool,
                    q,
                    qv.as_deref(),
                    remaining as i64,
                    false,
                    None,
                    None,
                )
                .await?;
                let ids: Vec<Uuid> = hits.iter().map(|h| h.id).collect();
                if ids.is_empty() {
                    vec![]
                } else {
                    let score_of: std::collections::HashMap<Uuid, f64> =
                        hits.iter().map(|h| (h.id, h.score)).collect();
                    let mut fetched: Vec<AtomDto> =
                        sqlx::query_as::<_, AtomDto>("SELECT * FROM atoms WHERE id = ANY($1)")
                            .bind(&ids)
                            .fetch_all(&self.pool)
                            .await?;
                    // 过期原子不注入（phase-2）：valid_until 已过 = 真记性不递过期记忆
                    let now = chrono::Utc::now();
                    fetched.retain(|a| a.valid_until.map(|vu| vu > now).unwrap_or(true));
                    // P10 新鲜度混排：final = 相关分 × 时间衰减（30 天半衰）——
                    // 老记忆不再凭旧高分挤掉新记忆；无 query 路径本就按新→旧。
                    fetched.sort_by(|a, b| {
                        let f = |x: &AtomDto| {
                            let age = (now - x.created_at).num_days().max(0) as f64;
                            score_of.get(&x.id).copied().unwrap_or(0.0) * (-age / 30.0).exp()
                        };
                        f(b).partial_cmp(&f(a)).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    fetched
                }
            }
            None => {
                sqlx::query_as(
                    "SELECT * FROM atoms WHERE status = 'active' AND NOT sensitive \
                 AND (valid_until IS NULL OR valid_until > now()) \
                 ORDER BY hit_count DESC, confidence DESC, created_at DESC LIMIT $1",
                )
                .bind(remaining as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };
        let mut out_atoms = Vec::new();
        for a in atoms {
            if count(&a.content, &mut chars_used, &mut truncated) {
                out_atoms.push(a);
            } else {
                truncated = true;
                break;
            }
        }

        // B9：context_pack 也是使用（AI 冷启动读路径），同样计热度；
        // no_feedback=true 供 harness 注入/测试使用——不刷热度（B6 污染防护）
        if !no_feedback {
            self.fire_hit_feedback("atoms", out_atoms.iter().map(|a| a.id).collect());
            self.fire_hit_feedback("scenarios", out_scenarios.iter().map(|s| s.id).collect());
        }

        // 人审代问（议题三）：队列里的低置信项带给 AI——下次对话顺口确认一句，
        // atom-patch 回写，人审从「翻网页」变「一句话」。不计热度。
        let pending_review: Vec<AtomDto> = sqlx::query_as(
            "SELECT * FROM atoms WHERE needs_review AND status = 'active' AND NOT sensitive ORDER BY created_at DESC LIMIT 5",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(ContextPack {
            persona,
            scenarios: out_scenarios,
            atoms: out_atoms,
            entities: out_entities,
            pending_review,
            meta: ContextMeta {
                chars_used,
                truncated,
                query: query.map(String::from),
            },
        })
    }
}

/// source_refs 中擦除指定会话（标记 erased，保留结构）。
fn mark_erased(refs: serde_json::Value, erased_id: Uuid) -> serde_json::Value {
    match refs {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|mut item| {
                    if item.get("session_id").and_then(|v| v.as_str())
                        == Some(erased_id.to_string().as_str())
                    {
                        item["erased"] = serde_json::Value::Bool(true);
                    }
                    item
                })
                .collect(),
        ),
        other => other,
    }
}
