//! 持久化层：sqlx PgPool 装配、迁移执行、仓储（Phase 1 起逐步充实）。
//!
//! 约定：
//! - `server/migrations/` 是 schema 唯一定义处，启动时自动执行。
//! - 仓储只被 `core` 调用，`api` 不直接摸本 crate。

pub mod migrate;
pub mod pool;

pub use migrate::{current_version, run_migrations};
pub use pool::{PoolConfig, connect_pool};
