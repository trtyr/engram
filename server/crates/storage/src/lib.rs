//! 持久化层：sqlx PgPool 装配、迁移执行、领域仓储与持久化模型。
//!
//! 分层约定（见 engram projects 域架构篇）：
//! - `server/migrations/` 是 schema 唯一定义处，启动时自动执行。
//! - 领域表的业务面读写收口在 `repo::*`：core 服务与 api/mcp 经由本 crate
//!   访问数据（ADR-19 改约：不再宣称全仓「唯一收口」；新代码的领域表业务面
//!   必须走 repo，存量渐进收敛，不追改）。
//! - 持久化模型（sqlx::FromRow 行类型）在 `models::*`；core 将其 re-export 保持
//!   `engram_core::<域>::<Dto>` 路径兼容。
//! - 边界：流水线型 crate（distill / wiki-engine / llm / search）按既有设计拥有
//!   各自管道内部的 SQL（job 处理器 / 检索只读路径），不属于业务面仓储；
//!   jobs 表的管理面只读/收尾操作在 `engram_jobs::admin`。
//! - 事务边界归仓储：跨语句事务（快照+更新、purge 等）整体落在 repo 函数内。
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

pub mod error;
pub mod migrate;
pub mod models;
pub mod pool;
pub mod repo;

pub use error::{StoreError, StoreResult};
pub use migrate::{current_version, run_migrations};
pub use pool::{PoolConfig, connect_pool};

/// 连接池类型再导出：上层（core）持池句柄无需直接依赖 sqlx。
pub use sqlx::PgPool;
