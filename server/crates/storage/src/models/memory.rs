//! 记忆域行类型（raw_sessions / atoms / scenarios / persona_aspects / entities…）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

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
    /// 断言强度：fact=用户明示/机器验证, inference=agent 推断, assumption=假设
    pub strength: String,
    /// 断言来源：user_stated/verified_probe/agent_inferred/doc
    pub source_kind: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// KV 值保值条目：value 逐字保存（蒸馏零介入），key 唯一 UPSERT 就地更新。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct KvEntryDto {
    pub id: Uuid,
    pub key: String,
    pub value: String,
    pub context: String,
    pub tags: Vec<String>,
    pub source: String,
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

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct GraphEdge {
    pub a: Uuid,
    pub b: Uuid,
    pub weight: i64,
}
