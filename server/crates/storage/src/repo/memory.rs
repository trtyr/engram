//! 记忆域仓储：raw_sessions / atoms / scenarios / persona_aspects / entities /
//! entity_relations / atom_entities / atom_revisions / entity_revisions 读写。
//!
//! 事务边界：purge_agent / purge_deep / merge_entities 整体在事务内封装。

mod atoms;
mod entity;
mod kv;
mod ops;
mod persona;
mod scenarios;
mod sessions;
pub use atoms::*;
pub use entity::*;
pub use kv::*;
pub use ops::*;
pub use persona::*;
pub use scenarios::*;
pub use sessions::*;

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::memory::{
    AtomDto, AtomRevision, EntityDto, EntityRelationDto, EntityRevision, GraphEdge, KvEntryDto,
    PersonaVersion, ScenarioDto, SessionDto, TimelineEvent,
};

// ---------- L0 会话 ----------

// ---------- L1 原子 ----------

// ---------- KV 值保值通道（蒸馏零介入——value 逐字保存） ----------

// ---------- L2 场景 ----------

// ---------- L3 画像 ----------

// ---------- 实体（记忆星系） ----------

// ---------- 实体关系 ----------

// ---------- 汇总/时间轴/审计 ----------
