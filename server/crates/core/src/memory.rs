//! 记忆域服务：L0 写入/触发、检索、上下文包、L1 治理、L3 画像视图。
//!
//! 持久化在 `engram_storage::repo::memory`（本文件只保留校验、语义判定与编排）。

use chrono::{DateTime, Utc};
use engram_jobs::types::Job;
use engram_jobs::{JobQueue, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::tokenize;
use engram_search::{SearchHit, search_atoms, search_scenarios};
use engram_storage::repo::memory as repo;
use engram_storage::{PgPool, StoreError};
use serde::Serialize;
use serde_json::{Value, json};
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

impl From<StoreError> for MemoryError {
    fn from(e: StoreError) -> Self {
        MemoryError::Storage(e.to_string())
    }
}

// ---------- DTO（api 直接复用，utoipa schema；行类型在 storage，此处 re-export） ----------

pub use engram_storage::models::memory::{
    AtomDto, AtomRevision, EntityDto, EntityRelationDto, EntityRevision, GraphEdge, KvEntryDto,
    PersonaVersion, ScenarioDto, SessionDto, TimelineEvent,
};

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
    /// 待审项（≤5 条）——AI 在对话中顺口确认后 atom-patch 回写
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

/// 批量遗忘操作（恢复/擦除所选）的结果：逐条成败互不影响。
#[derive(Debug, Default, Serialize, utoipa::ToSchema)]
pub struct BatchOutcome {
    pub succeeded: usize,
    pub failed: Vec<BatchFailure>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BatchFailure {
    pub id: Uuid,
    pub error: String,
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

/// 查询侧嵌入指令（Qwen3-Embedding 等非对称检索模型用）；None = 不包装。
/// KV 陈旧提示：updated_at 超过 KV_STALE_DAYS 天时附带提示（不阻塞使用——只是提醒）。
pub fn kv_stale_hint(mut e: KvEntryDto) -> KvEntryDto {
    let age = chrono::Utc::now() - e.updated_at;
    if age > chrono::Duration::days(engram_storage::models::memory::KV_STALE_DAYS) {
        e.stale_hint = Some(format!(
            "此值最后校验于 {}（超过 {} 天），可能已过期——引用前建议先实测",
            e.updated_at.format("%Y-%m-%d"),
            engram_storage::models::memory::KV_STALE_DAYS
        ));
    }
    e
}

static QUERY_INSTRUCTION: std::sync::LazyLock<Option<String>> = std::sync::LazyLock::new(|| {
    std::env::var("AGENT_MEMORY_EMBED_QUERY_INSTRUCTION")
        .ok()
        .filter(|s| !s.trim().is_empty())
});

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
/// 长文档应走 wiki 文档域 /wiki/upload 分块摄取。
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
        // D22：轮内 ts 显式提供时必须可解析（垃圾时间戳原样落库会污染时间检索/排序）；
        // 与 cursor/from/to 同一宽容口径：RFC3339 全形态或 date-only
        if let Some(ts) = t.get("ts").and_then(|v| v.as_str())
            && !ts.trim().is_empty()
            && chrono::DateTime::parse_from_rfc3339(ts.trim()).is_err()
            && chrono::NaiveDate::parse_from_str(ts.trim(), "%Y-%m-%d").is_err()
        {
            return Err(MemoryError::BadRequest(format!(
                "第 {} 轮 ts 无法解析（收到 {ts:?}）——期望 ISO8601（2026-09-02 或 2026-09-02T00:00:00Z）",
                i + 1
            )));
        }
    }
    Ok(())
}

/// distill 模式校验（D21）：拼错的值（如 "manaul"）静默落入 auto 语义会违背调用方意图，
/// 与同函数内 speaker/priority 的响亮拒绝同口径。allowed 按通道不同（append 无 manual）。
fn validate_distill(distill: &str, allowed: &[&str]) -> Result<(), MemoryError> {
    if !allowed.contains(&distill) {
        return Err(MemoryError::BadRequest(format!(
            "distill 仅接受 {}（收到 {distill:?}）",
            allowed.join("/")
        )));
    }
    Ok(())
}

/// deep purge 核心（供 MemoryService 与 API 的 deep_purge 定时 job 复用）。
pub async fn purge_deep_pool(
    pool: &PgPool,
) -> Result<serde_json::Value, engram_storage::StoreError> {
    repo::purge_deep(pool).await
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
        self.write_session_identity(agent, turns, distill, sensitive, None, None, None)
            .await
    }

    /// 带身份归因与幂等键的写入（公网多Agent P001 步骤1）。
    /// api_key_id/key_name 由请求凭据自动注入；client_ref 命中唯一索引时返回既有会话
    /// （幂等——网络重试不产生重复会话）。
    #[allow(clippy::too_many_arguments)]
    pub async fn write_session_identity(
        &self,
        agent: &str,
        turns: serde_json::Value,
        distill: &str,
        sensitive: bool,
        api_key_id: Option<Uuid>,
        key_name_snapshot: Option<&str>,
        client_ref: Option<&str>,
    ) -> Result<SessionDto, MemoryError> {
        validate_distill(distill, &["auto", "manual", "off"])?;
        // D 观察项：空串 agent 归一化（回读 agent="" 无意义）
        let agent = agent.trim();
        let agent = if agent.is_empty() { "unknown" } else { agent };
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("会话至少一轮".into()));
        }
        validate_turns(arr)?;
        let id = Uuid::now_v7();
        // v2 修复（H-A2）：distill=off 落 metadata 标记——蒸馏认领扫描据此豁免，
        // off 会话不再被后续任何 extract 任务的 pending 全量扫描"顺走"蒸掉。
        // off 是会话级永久语义（直到 void/erase），append 不改变它。
        let metadata = if distill == "off" {
            json!({"distill": "off"})
        } else {
            json!({})
        };
        let row = repo::insert_session_identity(
            &self.pool,
            id,
            agent,
            &turns,
            sensitive,
            &metadata,
            api_key_id,
            key_name_snapshot,
            client_ref,
        )
        .await?;

        // LLM 未配置时显式暴露（MCP 黑盒测试 D1：蒸馏静默失败不可接受——
        // manual 最应显式失败；auto 已入库但提示不会蒸馏）
        if distill != "off"
            && let Err(e) = self
                .registry
                .resolve(engram_llm::types::Purpose::Extract)
                .await
        {
            let msg = format!(
                "LLM 未配置或不可用（{e}）——蒸馏无法执行。请管理员在「设置 → AI 功能」配置模型后重试"
            );
            if distill == "manual" {
                return Err(MemoryError::LlmNotConfigured(msg));
            }
            tracing::warn!("{msg}（auto 会话已入库，distill_status 保持 pending）");
        }

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
        validate_distill(distill, &["auto", "manual", "off"])?;
        let turns = Self::parse_import(content, format)?;
        // M-1/SEC-B 同口径：导入的 turns 也过校验（jsonl 行 content 为空同样挡）
        if let Some(arr) = turns.as_array() {
            validate_turns(arr)?;
        }
        let id = Uuid::now_v7();
        let row = repo::insert_session_import(&self.pool, id, agent, &turns).await?;
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
        // append 无 manual 语义（off 会话本就被豁免）；乱传 manual 此前被静默忽略
        validate_distill(distill, &["auto", "off"])?;
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("追加至少一轮".into()));
        }
        validate_turns(arr)?;
        let cur = repo::find_session(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))?;
        if cur.distill_status != "pending" {
            return Err(MemoryError::BadRequest(format!(
                "会话已蒸馏（{}），不可追加——请开新会话",
                cur.distill_status
            )));
        }
        let row = repo::append_session_update(
            &self.pool,
            id,
            &serde_json::Value::Array(arr.to_vec()),
            agent,
        )
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
        if limit < 0 {
            return Err(MemoryError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        Ok(repo::list_sessions(&self.pool, agent, cursor, limit.min(200)).await?)
    }

    /// 会话列表轻量行（浏览/定位用）：轮次数 + 首条消息预览，不含正文数组。
    /// MCP memory_list_sessions 用——按需取用而非截断全文。
    pub async fn list_sessions_meta(
        &self,
        agent: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Value>, MemoryError> {
        if limit < 0 {
            return Err(MemoryError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        Ok(repo::list_sessions_meta(&self.pool, agent, cursor, limit.min(200)).await?)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        repo::find_session(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))
    }

    /// L0 擦除：删会话 + 引用它的 atoms 标记来源失效。
    /// erase 是「物理删除」语义——派生原子也必须撤出检索：删除前把源自该会话的
    /// active/superseded 原子级联归档（与 void/purge_agent 同口径；多源原子同被归档，
    /// 保守可恢复）。此前只标 source_refs erased，原子仍在检索里，违背遗忘承诺。
    pub async fn erase_session(&self, id: Uuid) -> Result<(), MemoryError> {
        let archived = repo::archive_atoms_by_session(&self.pool, &id.to_string()).await?;
        let affected = repo::delete_session(&self.pool, id).await?;
        if affected == 0 {
            return Err(MemoryError::NotFound(format!("会话 {id} 不存在")));
        }
        // 来源失效标记：source_refs 里含该会话的原子加 erased 标记
        let atoms = repo::list_atom_refs_like(&self.pool, id).await?;
        for (aid, refs) in atoms {
            let marked = mark_erased(refs, id);
            repo::update_atom_source_refs(&self.pool, aid, &marked).await?;
        }
        if archived > 0 {
            self.audit(
                "session_erase_cascade",
                json!({"session": id.to_string(), "archived_atoms": archived}),
            )
            .await;
        }
        // 与 void 同一套孤儿清扫（2026-09-08 用户：画像空了圈子里怎么还有东西）——
        // erase 归档原子后，挂链原子全部失效的实体也要退场，否则圈子和画像口径分裂。
        // 场景收敛一并触发（快照里可能引用被归档的成员）。
        self.queue
            .enqueue(
                JobTemplate::new("organize_scenarios")
                    .with_payload(json!({ "converge_only": true }))
                    .with_idempotency_key(format!("erase-converge-{id}")),
            )
            .await
            .ok();
        repo::archive_orphan_entities(&self.pool).await?;
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

    /// 手动触发蒸馏（AI 记忆管家）：撞车守卫——running extract_atoms 存在时只提示不投递
    /// （任务列表不留空跑记录）。mode："distill"（默认）/ "rebuild"（画像全量重建）/
    /// "sleep"（预留——内置节律线后开放）。
    pub async fn trigger_distill_manual(
        &self,
        full: bool,
        mode: &str,
        by: &str,
    ) -> Result<serde_json::Value, MemoryError> {
        if mode == "sleep" {
            return Err(MemoryError::BadRequest(
                "睡眠（记忆巩固）尚未上线——依赖内置节律线，敬请期待".into(),
            ));
        }
        if mode == "rebuild" {
            // 画像全量重建（收录哲学线 task-10）：不入蒸馏链，直接投递 distill_persona
            // 全量重建任务（payload.full_rebuild → persona.rs 以全部场景重算所有非钉住分面）。
            // 撞车守卫：running 的 distill_persona 存在时只提示不投递（与 extract 同模）。
            let running = repo::count_running_persona(&self.pool).await?;
            if running > 0 {
                return Ok(serde_json::json!({
                    "already_running": true,
                    "hint": "画像全量重建进行中——等它完成看效果，再决定是否重来",
                }));
            }
            let job = self
                .queue
                .enqueue(
                    engram_jobs::JobTemplate::new("distill_persona")
                        .with_payload(serde_json::json!({"full_rebuild": true}))
                        .with_idempotency_key(format!(
                            "persona-rebuild-{}",
                            chrono::Utc::now().format("%Y%m%d%H%M")
                        )),
                )
                .await
                .map_err(|e| MemoryError::Storage(e.to_string()))?;
            return Ok(serde_json::json!({
                "already_running": false,
                "hint": "画像全量重建已触发：以全部场景重算所有非钉住分面（manually_edited 豁免）",
                "jobs": [{ "id": job.id.to_string(), "kind": job.kind }],
            }));
        }
        if mode != "distill" {
            return Err(MemoryError::BadRequest(format!(
                "mode 仅支持 distill / sleep（收到 {mode}）"
            )));
        }
        let running = repo::count_running_extract(&self.pool).await?;
        if running > 0 {
            return Ok(serde_json::json!({
                "already_running": true,
                "running": running,
                "hint": "当前正在蒸馏中——等它完成看看效果，再决定是否手动触发",
            }));
        }
        let jobs = self.trigger_distill(full, "manual", by).await?;
        let list: Vec<serde_json::Value> = jobs
            .iter()
            .map(|j| serde_json::json!({ "id": j.id.to_string(), "kind": j.kind }))
            .collect();
        Ok(serde_json::json!({
            "already_running": false,
            "hint": if full {
                "已触发蒸馏 + 全量整理（consolidate）"
            } else {
                "已触发蒸馏链（抽取→仲裁→归组→画像）"
            },
            "jobs": list,
        }))
    }

    /// 节律状态（memory-rhythm）：外部 cron 的心跳与积压年龄，供设置页判定逾期。
    /// last_heartbeat 复用 jobs 审计行（kind=rhythm_heartbeat）；pending 统计扫
    /// raw_sessions 积压（cron 兜底蒸馏的对象）。
    pub async fn rhythm_status(&self) -> Result<serde_json::Value, MemoryError> {
        let heartbeat = repo::rhythm_last_heartbeat(&self.pool).await?;
        let (count, oldest) = repo::session_backlog(&self.pool).await?;
        let age_secs = oldest.map(|t| (Utc::now() - t).num_seconds());
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
        if limit < 0 {
            return Err(MemoryError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        Ok(repo::list_atoms(
            &self.pool,
            kind,
            status,
            needs_review,
            cursor,
            limit.min(500),
        )
        .await?)
    }

    /// 手工新增（待审补充；active 直接入库）。
    #[allow(clippy::too_many_arguments)]
    pub async fn create_atom(
        &self,
        kind: &str,
        content: &str,
        confidence: f32,
        occurred_at: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
        sensitive: bool,
        strength: Option<&str>,
        source_kind: Option<&str>,
    ) -> Result<AtomDto, MemoryError> {
        const STRENGTHS: [&str; 3] = ["fact", "inference", "assumption"];
        const SOURCES: [&str; 4] = ["user_stated", "verified_probe", "agent_inferred", "doc"];
        if let Some(v) = strength
            && !STRENGTHS.contains(&v)
        {
            return Err(MemoryError::BadRequest(format!(
                "strength 仅接受 fact/inference/assumption（收到 {v}）"
            )));
        }
        if let Some(v) = source_kind
            && !SOURCES.contains(&v)
        {
            return Err(MemoryError::BadRequest(format!(
                "source 仅接受 user_stated/verified_probe/agent_inferred/doc（收到 {v}）"
            )));
        }
        let text = content.trim();
        // 输入校验：空内容 + 超长（对齐蒸馏链的 1~120 字契约）
        if text.is_empty() {
            return Err(MemoryError::BadRequest("原子内容不能为空".into()));
        }
        if text.chars().count() > 120 {
            return Err(MemoryError::BadRequest(format!(
                "原子内容超长：最多 120 字，当前 {} 字——原子只收一句话；成段内容走 write_session（蒸馏自动切分），remember 用默认蒸馏路径（不带 strength=fact）",
                text.chars().count()
            )));
        }
        // A4 幂等护栏：同 kind + 同内容（trim 后）的 active 原子已存在则直接返回它——
        // AI 重试/重复直写不会双份（2026-08-31 测试方实测两条一模一样的生日原子）。
        // 近重复的语义合并仍归 arbitrate/consolidate，这里只挡精确重复。
        if let Some(existing) = repo::find_active_atom(&self.pool, kind, text).await? {
            tracing::info!(atom_id = %existing.id, "直写命中已有同内容原子，幂等返回");
            return Ok(existing);
        }
        // A1：与蒸馏链同规则——置信 <0.55 自动进待审，不直接生效污染记忆库
        // （此前直写硬编码 needs_review=false，文档/CLI 提示/实现三方打架）。
        let needs_review = confidence < 0.55;
        let id = Uuid::now_v7();
        let emb = self.try_embed(&[text.to_string()]).await;
        let row = repo::insert_atom(
            &self.pool,
            id,
            kind,
            text,
            confidence,
            needs_review,
            sensitive,
            occurred_at,
            valid_until,
            emb.and_then(|v| v.into_iter().next()),
            &engram_search::tokenize::tsv_text(text),
            strength.unwrap_or("fact"),
            source_kind.unwrap_or("user_stated"),
        )
        .await?;
        Ok(row)
    }

    /// correct 快路径（AI 记忆管家，收录哲学线）：单事务取代链——
    /// 新原子 active（继承 target 的 kind，confidence 0.95/fact/user_stated），
    /// 旧原子 superseded + superseded_by 指针。治理：仅 active 且非 sensitive 的目标。
    pub async fn correct_atom(&self, target_id: Uuid, text: &str) -> Result<AtomDto, MemoryError> {
        let cur = repo::find_atom(&self.pool, target_id)
            .await?
            .ok_or_else(|| {
                MemoryError::NotFound(format!("目标原子 {target_id} 不存在——先 search 定位再更正"))
            })?;
        if cur.status != "active" {
            return Err(MemoryError::BadRequest(format!(
                "目标原子已是 {} 状态，仅 active 原子可被取代——先 search 找最新条目",
                cur.status
            )));
        }
        if cur.sensitive {
            return Err(MemoryError::BadRequest(
                "敏感原子禁走 correct 快路径——相关更正走会话蒸馏通道".into(),
            ));
        }
        let t = text.trim();
        if t.is_empty() {
            return Err(MemoryError::BadRequest("更正内容不能为空".into()));
        }
        if t.chars().count() > 120 {
            return Err(MemoryError::BadRequest(format!(
                "更正内容超长：最多 120 字，当前 {} 字",
                t.chars().count()
            )));
        }
        let new_id = Uuid::now_v7();
        let emb = self.try_embed(&[t.to_string()]).await;
        let row = repo::correct_atom(
            &self.pool,
            new_id,
            target_id,
            &cur.kind,
            t,
            emb.and_then(|v| v.into_iter().next()),
            &engram_search::tokenize::tsv_text(t),
        )
        .await?
        .ok_or_else(|| {
            MemoryError::BadRequest(
                "目标原子已被并发变更（非 active 或敏感）——重新 search 后再试".into(),
            )
        })?;
        self.audit(
            "correct_atom",
            json!({
                "target": target_id.to_string(),
                "new": row.id.to_string(),
                "old_content": cur.content,
                "new_content": t,
            }),
        )
        .await;
        Ok(row)
    }

    /// 待审复核 confirm：摘 needs_review 标记（AI 代管复核，仅 needs_review=true 可处置）。
    pub async fn confirm_review(&self, id: Uuid) -> Result<AtomDto, MemoryError> {
        let row = repo::review_confirm(&self.pool, id).await?.ok_or_else(|| {
            MemoryError::NotFound(format!(
                "原子 {id} 不存在或不在待审状态——confirm 仅可处置 needs_review=true 的条目"
            ))
        })?;
        self.audit(
            "review_confirm",
            json!({ "atom_id": id.to_string(), "content": row.content }),
        )
        .await;
        Ok(row)
    }

    /// 待审复核 discard：归档（仅 needs_review=true 且 active）；归档触发场景快照收敛。
    pub async fn discard_review(&self, id: Uuid) -> Result<AtomDto, MemoryError> {
        let row = repo::review_discard(&self.pool, id).await?.ok_or_else(|| {
            MemoryError::NotFound(format!(
                "原子 {id} 不存在或不在待审状态——discard 仅可处置 needs_review=true 的条目"
            ))
        })?;
        self.audit(
            "review_discard",
            json!({ "atom_id": id.to_string(), "content": row.content }),
        )
        .await;
        let bucket = chrono::Utc::now().timestamp() / self.debounce_secs;
        let _ = self
            .queue
            .enqueue(
                JobTemplate::new("organize_scenarios")
                    .with_idempotency_key(format!("snapshot-refresh-{bucket}"))
                    .with_payload(
                        serde_json::json!({"converge_only": true, "atom_id": id.to_string()}),
                    )
                    .with_due(chrono::Utc::now() + chrono::Duration::seconds(self.debounce_secs)),
            )
            .await;
        Ok(row)
    }

    /// 蒸馏回执：一次会话蒸馏产出了什么（原子 id + 内容预览 + 状态）。
    pub async fn distill_result(&self, session_id: Uuid) -> Result<serde_json::Value, MemoryError> {
        let Some((distill_status, metadata)) =
            repo::session_distill_meta(&self.pool, session_id).await?
        else {
            return Err(MemoryError::NotFound(format!("会话 {session_id} 不存在")));
        };
        let status = if metadata.get("distill").and_then(|v| v.as_str()) == Some("off") {
            "off（该会话标记为不蒸馏）".to_string()
        } else {
            distill_status
        };
        let atoms = repo::atoms_by_session(&self.pool, session_id).await?;
        let preview: Vec<serde_json::Value> = atoms
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "kind": a.kind,
                    "content": a.content,
                    "strength": a.strength,
                    "source_kind": a.source_kind,
                    "status": a.status,
                    "needs_review": a.needs_review,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "session_id": session_id,
            "distill_status": status,
            "atom_count": atoms.len(),
            "atoms": preview,
        }))
    }

    // ---------- KV 值保值通道（蒸馏零介入——value 逐字保存） ----------

    /// 写入/更新一个结构化值。同 key 就地覆盖（可变状态不产生取代链）。
    pub async fn kv_put(
        &self,
        key: &str,
        value: &str,
        context: Option<&str>,
        tags: Option<Vec<String>>,
        source: Option<&str>,
    ) -> Result<KvEntryDto, MemoryError> {
        let k = key.trim();
        if k.is_empty() || k.chars().count() > 200 {
            return Err(MemoryError::BadRequest("key 不能为空且 ≤200 字符".into()));
        }
        let v = value;
        if v.is_empty() {
            return Err(MemoryError::BadRequest(
                "value 不能为空——KV 是精确值通道，不放空话".into(),
            ));
        }
        const SOURCES: [&str; 4] = ["user_stated", "verified_probe", "agent_inferred", "doc"];
        let src = source.unwrap_or("user_stated");
        if !SOURCES.contains(&src) {
            return Err(MemoryError::BadRequest(format!(
                "source 仅接受 user_stated/verified_probe/agent_inferred/doc（收到 {src}）"
            )));
        }
        let row = repo::kv_upsert(
            &self.pool,
            k,
            v,
            context.unwrap_or("").trim(),
            &tags.unwrap_or_default(),
            src,
        )
        .await?;
        Ok(row)
    }

    pub async fn kv_get(&self, key: &str) -> Result<Option<KvEntryDto>, MemoryError> {
        Ok(repo::kv_get(&self.pool, key.trim())
            .await?
            .map(kv_stale_hint))
    }

    pub async fn kv_list(&self, limit: i64) -> Result<Vec<KvEntryDto>, MemoryError> {
        Ok(repo::kv_list(&self.pool, limit.clamp(1, 500))
            .await?
            .into_iter()
            .map(kv_stale_hint)
            .collect())
    }

    /// 字面量直查（key/value/context ILIKE）——精确值不依赖分词。
    pub async fn kv_search(&self, query: &str, limit: i64) -> Result<Vec<KvEntryDto>, MemoryError> {
        let q = query.trim();
        if q.chars().count() < 3 {
            return Err(MemoryError::BadRequest(
                "检索词至少 3 字符（防全表扫短串）".into(),
            ));
        }
        Ok(repo::kv_search_literal(&self.pool, q, limit.clamp(1, 100))
            .await?
            .into_iter()
            .map(kv_stale_hint)
            .collect())
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
        let cur = repo::find_atom(&self.pool, id)
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
            repo::insert_atom_revision(
                &self.pool,
                Uuid::now_v7(),
                id,
                &cur.content,
                &cur.kind,
                cur.confidence,
                actor,
            )
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

        let row = repo::update_atom_full(
            &self.pool,
            id,
            &new_content,
            new_conf,
            new_status,
            needs_review,
            superseded_by,
            occurred_at,
            valid_until,
            sensitive,
            emb.and_then(|v| v.into_iter().next()),
            &engram_search::tokenize::tsv_text(&new_content),
            new_kind,
        )
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
        Ok(repo::list_scenarios(&self.pool, limit.min(200)).await?)
    }

    pub async fn get_scenario(&self, id: Uuid) -> Result<ScenarioDto, MemoryError> {
        repo::find_scenario(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("场景 {id} 不存在")))
    }

    /// 当前画像（每分面最新版）。
    pub async fn persona(&self) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(repo::persona_current(&self.pool).await?)
    }

    /// 分面版本历史。
    pub async fn persona_history(&self, aspect: &str) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(repo::persona_history(&self.pool, aspect).await?)
    }

    /// 回滚分面到历史版本（以新版本号落地当前内容——历史不可变）。
    /// 编辑分面（version+1 落钉：manually_edited=true，蒸馏产出对该分面落库前被守卫丢弃）。
    /// 编辑能力：用户 Web 编辑与 AI persona_edit（收录哲学线授权的执行器）共用此路；
    /// 钉住 = 蒸馏绕开，解冻 = persona_unpin。
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
        let next_v = repo::persona_max_version(&self.pool, aspect)
            .await?
            .flatten()
            .map(|v| v + 1)
            .unwrap_or(1);
        repo::insert_persona_pinned(&self.pool, Uuid::now_v7(), aspect, content, next_v).await?;
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
        repo::persona_unpin(&self.pool, aspect).await?;
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
        Ok(repo::atom_revisions(&self.pool, atom_id).await?)
    }

    /// 实体摘要版本链（圈子强化）：手编档案的历史，最近在前。
    pub async fn entity_revisions(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<EntityRevision>, MemoryError> {
        Ok(repo::entity_revisions(&self.pool, entity_id).await?)
    }

    /// 实体关系列表（有向类型化；可选按实体过滤 from/to 两端）。
    pub async fn list_relations(
        &self,
        entity_id: Option<Uuid>,
    ) -> Result<Vec<EntityRelationDto>, MemoryError> {
        let rows = match entity_id {
            Some(eid) => repo::entity_relations(&self.pool, eid).await?,
            None => repo::list_relations(&self.pool).await?,
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
        let row =
            repo::insert_relation(&self.pool, Uuid::now_v7(), from, to, rel_type, source).await?;
        Ok(row)
    }

    /// 删关系。
    pub async fn delete_relation(&self, id: Uuid) -> Result<(), MemoryError> {
        let n = repo::delete_relation(&self.pool, id).await?;
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
        let (content, v) = repo::persona_latest(&self.pool, aspect)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("分面 {aspect} 不存在")))?;
        repo::insert_persona_pinned(&self.pool, Uuid::now_v7(), aspect, &content, v + 1).await?;
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
        let target = repo::persona_version(&self.pool, aspect, to_version)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("版本 {aspect}#{to_version} 不存在")))?;

        let next_v = repo::persona_max_version(&self.pool, aspect)
            .await?
            .flatten()
            .map(|v| v + 1)
            .unwrap_or(1);

        repo::insert_persona_rollback(
            &self.pool,
            Uuid::now_v7(),
            aspect,
            &target.content,
            next_v,
            &json!({"rollback_to": to_version}),
        )
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
        Ok(repo::list_entities(&self.pool, kind).await?)
    }

    async fn entity_row(&self, id: Uuid) -> Result<EntityDto, MemoryError> {
        repo::entity_row(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("实体 {id} 不存在")))
    }

    /// 实体详情：画像摘要 + 相关原子时间线 + 相关场景。
    pub async fn get_entity(&self, id: Uuid) -> Result<EntityDetail, MemoryError> {
        let entity = self.entity_row(id).await?;
        let atoms = repo::entity_atoms(&self.pool, id).await?;
        let scenarios = repo::entity_scenarios(&self.pool, id).await?;
        let neighbors = repo::entity_neighbors(&self.pool, id).await?;
        let relations = repo::entity_relations(&self.pool, id).await?;
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
        let dup = repo::entity_id_by_name_kind(&self.pool, name, kind).await?;
        if dup.is_some() {
            return Err(MemoryError::BadRequest(format!(
                "同名同类实体已存在：{name}"
            )));
        }
        repo::insert_entity(&self.pool, Uuid::now_v7(), name, kind, summary).await?;
        let id = repo::entity_id_by_name_kind(&self.pool, name, kind)
            .await?
            .expect("刚插入的实体必能查到");
        self.entity_row(id).await
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
            repo::update_entity_name(&self.pool, id, n).await?;
            changed = true;
        }
        if let Some(s) = summary {
            // 版本链：旧摘要进 entity_revisions（轻量历史，复用原子 revisions 模式）
            if let Some(old_summary) = repo::entity_summary(&self.pool, id).await?
                && old_summary != s
            {
                repo::insert_entity_revision(&self.pool, Uuid::now_v7(), id, &old_summary, actor)
                    .await?;
            }
            repo::update_entity_summary(&self.pool, id, s).await?;
            changed = true;
        }
        if changed {
            // 用户手编实体档案 → 钉住（consolidate 档案重生成绕开）；审计
            repo::pin_entity(&self.pool, id).await?;
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
        let n = repo::delete_entity(&self.pool, id).await?;
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
    /// archived/superseded 等非 active 原子不动（本来就是历史）；待审候选一并归档。
    pub async fn forget_entity(&self, id: Uuid) -> Result<usize, MemoryError> {
        let cur = repo::live_entity_id(&self.pool, id).await?;
        if cur.is_none() {
            return Err(MemoryError::NotFound(format!("实体 {id} 不存在或已合并")));
        }
        let n = repo::archive_atoms_by_entity(&self.pool, id).await? as usize;
        repo::detach_entity_links(&self.pool, id).await?;
        self.delete_entity(id).await?;
        Ok(n)
    }

    /// P5 会话作废：「这段白记了」——标记 void，蒸馏跳过（claim 只取 pending）。
    /// 只允许 pending 会话作废（已蒸馏的产出用 purge 清场处理）。
    /// M-2（2026-09-03）：「不存在」404 与「非 pending」400 分开报，不再合并一句。
    /// P5 会话作废（v2 扩大语义）：「这段白记了」。
    /// - pending/off 会话：标记 void，蒸馏跳过（原文保留可审计）；
    /// - done 会话：void + **级联归档**其蒸馏产出的 active 原子（v2 修复 P0-3 遗忘断层——
    ///   此前已蒸馏会话无任何遗忘手段，错误文案指路的 purge 又不在 MCP 工具面）。
    ///   归档保留原文与原子（可审计、可恢复），检索/context 不再返回。
    /// - processing：仍拒绝（蒸馏 worker 持有中，稍后重试）。
    pub async fn void_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        let status = repo::session_distill_status(&self.pool, id).await?;
        match status.as_deref() {
            None => return Err(MemoryError::NotFound(format!("会话 {id} 不存在"))),
            Some("processing") => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 正在蒸馏（processing）——请稍后重试作废"
                )));
            }
            Some("void") => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 已处理（当前状态 void）——已作废，无需重复操作"
                )));
            }
            _ => {}
        }
        let row = repo::void_session_update(&self.pool, id).await?;
        let row = row.ok_or_else(|| {
            // 查询与更新之间的竞态兜底（状态刚被蒸馏 worker 抢走）
            MemoryError::BadRequest(format!(
                "会话 {id} 刚被蒸馏任务取走（processing）——请稍后重试作废"
            ))
        })?;
        // done 会话：级联归档其蒸馏产出的 active 原子（与 purge_agent 同口径）
        if status.as_deref() == Some("done") {
            let archived = repo::archive_atoms_by_session(&self.pool, &id.to_string()).await?;
            // 审计链：job 行记遗忘动作（与 purge/erase 同哲学）
            self.audit(
                "session_void_cascade",
                json!({"session": id.to_string(), "archived_atoms": archived as i64}),
            )
            .await;
        }
        // D15/D16：遗忘级联（任何 void 都触发；converge 与孤儿归档均幂等）——
        // L2 场景收敛 + L3/实体层的孤儿回收，保证「遗忘」后各层即时干净。
        // 历史遗留的孤儿实体（修复前产生）也能借此回收
        self.queue
            .enqueue(
                JobTemplate::new("organize_scenarios")
                    .with_payload(json!({ "converge_only": true }))
                    .with_idempotency_key(format!("void-converge-{id}")),
            )
            .await
            .ok();
        repo::archive_orphan_entities(&self.pool).await?;
        Ok(row)
    }

    /// 恢复 void 会话（forget mode="restore"，R 报告建议 #7：void 原文保留，恢复成本低）：
    /// - distill_status 还原自作废时的存档（pending→pending 可继续蒸馏，done→done）；
    /// - 级联归档的原子一并恢复（superseded_by 在 → superseded；否则 → active）。
    /// - 返回 (恢复的会话, 恢复的原子数)。
    pub async fn unvoid_session(&self, id: Uuid) -> Result<(SessionDto, u64), MemoryError> {
        let status = repo::session_distill_status(&self.pool, id).await?;
        match status.as_deref() {
            None => return Err(MemoryError::NotFound(format!("会话 {id} 不存在"))),
            Some("processing") => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 正在蒸馏（processing）——等蒸馏完成后再作废/恢复"
                )));
            }
            Some("void") => {}
            Some(other) => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 未被作废（当前状态 {other}）——restore 只对 void 会话有意义"
                )));
            }
        }
        let row = repo::unvoid_session_update(&self.pool, id)
            .await?
            .ok_or_else(|| {
                MemoryError::BadRequest(format!("会话 {id} 刚被蒸馏任务取走——请稍后重试"))
            })?;
        let restored = repo::restore_atoms_by_session(&self.pool, &id.to_string()).await?;
        self.audit(
            "session_unvoid_restore",
            json!({"session": id.to_string(), "restored_atoms": restored, "to": row.distill_status}),
        )
        .await;
        // 还原成 pending 的会话恢复蒸馏资格——触发一次防抖扫描（off 豁免由 metadata 保留）
        if row.distill_status == "pending" {
            engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                .await
                .ok();
        }
        // 恢复的 active 原子需要场景收敛重算（幂等）
        if restored > 0 {
            self.queue
                .enqueue(
                    JobTemplate::new("organize_scenarios")
                        .with_payload(json!({ "converge_only": true }))
                        .with_idempotency_key(format!("unvoid-converge-{id}")),
                )
                .await
                .ok();
        }
        Ok((row, restored))
    }

    /// 批量撤销作废（Web「恢复所选」）：逐条 unvoid，单条失败不影响其余——
    /// 混合选择（含非 void）时失败项逐条带原因返回，前端据此汇总提示。
    pub async fn unvoid_sessions(&self, ids: &[Uuid]) -> BatchOutcome {
        let mut out = BatchOutcome::default();
        for id in ids {
            match self.unvoid_session(*id).await {
                Ok(_) => out.succeeded += 1,
                Err(e) => out.failed.push(BatchFailure {
                    id: *id,
                    error: e.to_string(),
                }),
            }
        }
        out
    }

    /// 批量擦除（Web「擦除所选」）：逐条 erase_session（含原子级联），逐条成败互不影响。
    pub async fn erase_sessions(&self, ids: &[Uuid]) -> BatchOutcome {
        let mut out = BatchOutcome::default();
        for id in ids {
            match self.erase_session(*id).await {
                Ok(()) => out.succeeded += 1,
                Err(e) => out.failed.push(BatchFailure {
                    id: *id,
                    error: e.to_string(),
                }),
            }
        }
        out
    }

    /// P11/SEC-E 按 agent 清场（测试隔离，2026-09-03 彻底化）：该 agent **全部**会话
    /// 物理删除（pending/processing/done/void 一视同仁，sensitive 原文不留——
    /// 此前 done 会话残留曾导致敏感原始对话留库）+ 其产出的 active 原子归档（可恢复）。
    /// 返回 (erased_sessions, archived_atoms)。顺序敏感：先归档原子（JOIN 会话判归属）
    /// 再删会话——删会话后 JOIN 不可判归属。
    pub async fn purge_agent(&self, agent: &str) -> Result<(i64, i64), MemoryError> {
        Ok(repo::purge_agent_tx(&self.pool, agent).await?)
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
        repo::audit(&self.pool, kind, payload).await;
    }

    /// P4 全量导出（数据主权）：记忆域五表完整快照，JSON 随身带走。
    /// R4：sensitive 原子默认排除（隐私面不随导出扩大到文件系统），
    /// include_sensitive=true 显式包含——与检索 reveal 同权。
    pub async fn export(&self, include_sensitive: bool) -> Result<serde_json::Value, MemoryError> {
        let sessions = repo::list_all_sessions(&self.pool).await?;
        let atoms = repo::list_atoms_all(&self.pool, include_sensitive).await?;
        let scenarios = repo::list_scenarios_all(&self.pool).await?;
        let persona = repo::persona_all(&self.pool).await?;
        let entities = repo::entities_for_export(&self.pool).await?;
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
        let n = repo::count_atom(&self.pool, atom_id).await?;
        if n == 0 {
            return Err(MemoryError::NotFound(format!("原子 {atom_id} 不存在")));
        }
        repo::attach_atom_link(&self.pool, atom_id, entity_id).await?;
        repo::touch_entity(&self.pool, entity_id).await?;
        Ok(())
    }

    pub async fn detach_atom(&self, entity_id: Uuid, atom_id: Uuid) -> Result<(), MemoryError> {
        let n = repo::detach_atom_link(&self.pool, atom_id, entity_id).await?;
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
        let moved = repo::merge_entities_tx(&self.pool, from, into).await?;
        Ok(moved)
    }

    /// 星系图：节点（活体实体 + 密度）+ 共现边。
    pub async fn entity_graph(&self) -> Result<EntityGraph, MemoryError> {
        let nodes = self.list_entities(None).await?;
        let edges = repo::cooccurrence_edges(&self.pool).await?;
        let relations = self.list_relations(None).await?;
        Ok(EntityGraph {
            nodes,
            edges,
            relations,
        })
    }

    /// 全局记忆时间轴：原子（occurred_at 优先）/场景/实体按时间倒序合并。
    pub async fn timeline(&self, limit: i64) -> Result<Vec<TimelineEvent>, MemoryError> {
        Ok(repo::timeline(&self.pool, limit).await?)
    }

    /// 记忆域缺失向量统计（重嵌修复入口的状态面）。
    pub async fn embedding_status(&self) -> Result<EmbeddingStatus, MemoryError> {
        let (atoms_missing, scenarios_missing) = repo::embedding_missing_counts(&self.pool).await?;
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
        // L6：经记账门面（查询/场景嵌入计入用量）。
        // v2 遗留修复：embed 间歇失败（上游限流/抖动）此前被 .ok() 静默吞掉——
        // 查询无提示地降级为纯 FTS、文档静默缺向量。现在重试 3 次（退避）+ 失败显式记日志。
        for attempt in 1..=3 {
            match self
                .registry
                .embed_for(
                    Purpose::Embed,
                    texts.to_vec(),
                    Some(engram_distill::llm_port::embedding_dimensions()),
                    None,
                )
                .await
            {
                Ok(r) => return Some(r.embeddings),
                Err(e) => {
                    if attempt < 3 {
                        tracing::warn!(error = %e, attempt, "embed 失败，退避重试");
                        tokio::time::sleep(std::time::Duration::from_millis(400 * attempt as u64))
                            .await;
                    } else {
                        tracing::error!(
                            error = %e,
                            texts = texts.len(),
                            "embed 重试耗尽——查询降级为纯 FTS / 文档暂缺向量（后续 reembed 可补）"
                        );
                    }
                }
            }
        }
        None
    }

    /// v2（Qwen3-Embedding）：查询侧指令包装。Qwen3-Embedding 是非对称检索模型，
    /// 官方要求查询带 task instruction（文档侧不带）——不带指令的跨语言/短查询
    /// 召回会明显退化（实测：英文 pet 查询在阈值内找不到已有橘猫记忆）。
    /// 指令经 AGENT_MEMORY_EMBED_QUERY_INSTRUCTION 配置；未设置 = 不包装（bge-m3 等对称模型）。
    /// 文档侧（atom/scene 内容嵌入）必须保持不包装。
    async fn try_embed_query(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        match QUERY_INSTRUCTION.as_ref() {
            Some(instr) => {
                let wrapped: Vec<String> = texts
                    .iter()
                    .map(|t| format!("{instr}\nQuery: {t}"))
                    .collect();
                self.try_embed(&wrapped).await
            }
            None => self.try_embed(texts).await,
        }
    }

    /// B9 命中反馈：检索命中即异步回写 hit_count（best-effort，失败只记日志）。
    /// 不刷 updated_at——hit 是使用热度而非内容变化，避免扰动「最近更新」排序。
    fn fire_hit_feedback(&self, table: &'static str, ids: Vec<Uuid>) {
        if ids.is_empty() {
            return;
        }
        let pool = self.pool.clone();
        tokio::spawn(async move {
            if let Err(e) = repo::bump_hit_counts(&pool, table, &ids).await {
                tracing::warn!(error = %e, table, "hit_count 回写失败（不影响检索结果）");
            }
        });
    }

    /// 分层检索。无 embedding 通道时自动退化为纯 FTS。
    #[allow(clippy::too_many_arguments)]
    /// 2026-09-12 敏感口径放开：单用户系统全部记忆可见（用户拍板「全部放进来且能检索」）。
    /// sensitive 保留为**标记**（AtomDto.sensitive 字段照常返回），不再是隐身开关；
    /// reveal 参数移除——显式 hide 需求未来以 include_hidden 形态回归。
    pub async fn search(
        &self,
        query: &str,
        layers: &[&str],
        max_items: i64,
        no_feedback: bool,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<SearchResponse, MemoryError> {
        if max_items < 0 {
            return Err(MemoryError::BadRequest(format!(
                "max_items 不能为负（收到 {max_items}）"
            )));
        }
        let qv = self
            .try_embed_query(&[query.to_string()])
            .await
            .and_then(|v| v.first().cloned());
        let all = layers.is_empty();
        let want_e = all || layers.contains(&"entities");
        let want_l1 = all || layers.contains(&"l1");
        let want_l2 = all || layers.contains(&"l2");
        let want_l3 = all || layers.contains(&"l3");

        // 实体：token 命中（名字加权）——主角先行
        let entities = if want_e {
            engram_search::search_entities(&self.pool, query, max_items)
                .await
                .map_err(StoreError::from)?
        } else {
            vec![]
        };
        let mut l1 = if want_l1 {
            search_atoms(
                &self.pool,
                query,
                qv.as_deref(),
                max_items,
                true, // sensitive 口径放开（2026-09-12）——标记保留、不再隐身
                from,
                to,
            )
            .await
            .map_err(StoreError::from)?
        } else {
            vec![]
        };
        // ILIKE 补漏与 KV 权威通道（工单「库里有一搜必有」）：
        // - KV 是精确值唯一权威源——字面量命中**恒合并**进结果最前（FTS 噪音命中
        //   不应把权威值挤出结果），带 stale_hint
        // - atoms ILIKE 兜底仅在双腿零命中时触发（补漏，不打扰正常排序）
        if want_l1 && query.trim().chars().count() >= 3 {
            let mut kv_hits: Vec<engram_search::SearchHit> = Vec::new();
            if let Ok(kvs) =
                repo::kv_search_literal(&self.pool, query.trim(), max_items.max(1)).await
            {
                for kv in kvs {
                    let kv = kv_stale_hint(kv);
                    kv_hits.push(engram_search::SearchHit {
                        id: kv.id,
                        score: 100.0, // 权威源置顶
                        title: Some(kv.key.clone()),
                        snippet: match &kv.stale_hint {
                            Some(h) => format!("[kv:{}] {}\n⚠ {}", kv.key, kv.value, h),
                            None => format!("[kv:{}] {}", kv.key, kv.value),
                        },
                        kind: Some("kv".into()),
                        needs_review: None,
                    });
                }
            }
            if !kv_hits.is_empty() {
                kv_hits.extend(l1);
                l1 = kv_hits;
            } else if l1.is_empty() {
                for h in repo::atoms_literal_fallback(&self.pool, max_items, query, true).await? {
                    l1.push(engram_search::SearchHit {
                        id: h.id,
                        score: 0.01,
                        title: None,
                        snippet: h.content,
                        kind: Some(h.kind),
                        needs_review: Some(h.needs_review),
                    });
                }
            }
        }
        let l2 = if want_l2 {
            search_scenarios(&self.pool, query, qv.as_deref(), max_items)
                .await
                .map_err(StoreError::from)?
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

        // v2 修复（N7）：按**完整序列化体积**计量（含 evidence_refs/source_refs）——
        // 此前只数正文文本，chars_used 远小于真实注入体积，字符预算形同虚设。
        fn count_json(
            item: &impl serde::Serialize,
            budget_chars: usize,
            used: &mut usize,
            trunc: &mut bool,
        ) -> bool {
            let full = serde_json::to_string(item).unwrap_or_default();
            if *used + full.len() > budget_chars {
                *trunc = true;
                false
            } else {
                *used += full.len();
                true
            }
        }

        // 有 query 时预计算 query 向量（L2/L1 共用，避免重复 embed）
        let qv: Option<Vec<f32>> = match query {
            Some(q) => self
                .try_embed_query(&[q.to_string()])
                .await
                .and_then(|v| v.first().cloned()),
            None => None,
        };

        // L3 画像：v2 修复 N3（不计入 budget_items 条数）+ 字符子预算 ≤40%——
        // evidence_refs 计入计量后 7 个分面可能吃光整段预算，把 atoms/scenarios 挤成 0。
        // 画像先取（上限 40%），剩余预算全数让给场景/实体/原子。
        let persona_char_cap = budget_chars * 2 / 5;
        let persona: Vec<PersonaVersion> = self
            .persona()
            .await?
            .into_iter()
            .take_while(|p| count_json(&p, persona_char_cap, &mut chars_used, &mut truncated))
            .collect();

        // L2：有 query 按相关性，否则最近。v2（N3）：条数上限语义 = budget_items，
        // 保底 1——此前 budget_items*2/5 在小预算下把场景挤成 0。
        let scenario_cap = (budget_items * 2 / 5).max(1).min(budget_items.max(1));
        let scenarios = if let Some(q) = query {
            search_scenarios(&self.pool, q, qv.as_deref(), (scenario_cap as i64).max(3))
                .await
                .map_err(StoreError::from)?
        } else {
            self.list_scenarios((scenario_cap as i64).max(3))
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
        for h in scenarios.into_iter().take(scenario_cap) {
            match self.get_scenario(h.id).await {
                Ok(s) if count_json(&s, budget_chars, &mut chars_used, &mut truncated) => {
                    out_scenarios.push(s)
                }
                _ => break,
            }
        }

        // 实体透镜（小预算 ~20%）：AI 冷启动要知道用户世界里都有谁。
        // 有 query 走 token 相关（纯 jieba，不依赖向量）；无 query 按密度头部。best-effort。
        let ent_budget = (budget_items / 5).max(2).min(budget_items.max(1));
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
            let rows = repo::entities_by_ids(&self.pool, &entity_ids).await?;
            // 保持命中序（query 路径相关性优先；无 query 路径密度优先）
            let by_id: std::collections::HashMap<Uuid, EntityDto> =
                rows.into_iter().map(|e| (e.id, e)).collect();
            for id in entity_ids {
                if let Some(e) = by_id.get(&id).cloned() {
                    if count_json(&e, budget_chars, &mut chars_used, &mut truncated) {
                        out_entities.push(e);
                    } else {
                        truncated = true;
                        break;
                    }
                }
            }
        }

        // L1：v2 修复（N3）——条数上限 = budget_items（各层独立预算，不再被 persona/场景/实体
        // 相减挤成 0）；字符预算仍全局统一裁剪
        let remaining = budget_items;
        let atoms: Vec<AtomDto> = match query {
            Some(q) => {
                let hits = search_atoms(
                    &self.pool,
                    q,
                    qv.as_deref(),
                    remaining as i64,
                    true, // sensitive 口径放开（2026-09-12）
                    None,
                    None,
                )
                .await
                .map_err(StoreError::from)?;
                let ids: Vec<Uuid> = hits.iter().map(|h| h.id).collect();
                if ids.is_empty() {
                    vec![]
                } else {
                    let score_of: std::collections::HashMap<Uuid, f64> =
                        hits.iter().map(|h| (h.id, h.score)).collect();
                    let mut fetched = repo::atoms_by_ids(&self.pool, &ids).await?;
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
            None => repo::recent_active_atoms(&self.pool, remaining as i64).await?,
        };
        let mut out_atoms = Vec::new();
        for a in atoms {
            if count_json(&a, budget_chars, &mut chars_used, &mut truncated) {
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

        // 待审代问（议题三）：队列里的低置信项带给 AI——下次对话顺口确认一句，
        // atom-patch 回写，待审从「翻网页」变「一句话」。不计热度。
        let pending_review = repo::pending_review_atoms(&self.pool).await?;

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
