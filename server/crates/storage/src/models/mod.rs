//! 持久化模型（sqlx::FromRow 行类型）。api/mcp 的 utoipa schema 直接复用这些类型。

pub mod asset;
pub mod keys;
pub mod memory;
pub mod project;
pub mod skills;
pub mod wiki_docs;

pub mod wiki_promotions;
