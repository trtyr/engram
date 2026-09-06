//! 领域仓储模块。每个子模块对应一张（组）表的业务面读写。
//!
//! 约定：
//! - 函数自由组合（不持有状态），一律以 `&PgPool` 起参；事务整体封装在函数内。
//! - 「不存在」返回 `Option`，由服务层判定为各自的 NotFound。
//! - SQL 失败统一 [`StoreError`](crate::StoreError)；UNIQUE 冲突（23505）映射
//!   [`StoreError::Conflict`](crate::StoreError)（用 [`crate::error::is_unique_violation`] 判定）。

pub mod keys;
pub mod llm;
pub mod memory;
pub mod project;
pub mod settings;
pub mod skills;
pub mod transfer;
pub mod wiki_docs;
