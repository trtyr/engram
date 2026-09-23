//! 记忆域服务：L0 写入/触发、检索、上下文包、L1 治理、L3 画像视图。
//!
//! 持久化在 `engram_storage::repo::memory`（本文件只保留校验、语义判定与编排）。

use chrono::{DateTime, Utc};

mod atoms;
mod entity;
mod ops;
mod search;
mod sessions;

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

/// 单条原子内容上限（字符数）。2026-09-23 用户拍板 120 → 500（一句话事实的长度口径放宽）；
/// distill 的 extract_model.rs 同名常量与本值配对（distill 不依赖 core，两处人工同步）。
pub const ATOM_MAX_CHARS: usize = 500;

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
