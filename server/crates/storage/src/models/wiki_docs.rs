//! wiki 文档域行类型（wiki_documents / wiki_chunks）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DocumentDto {
    pub id: Uuid,
    pub title: String,
    pub source_uri: String,
    pub mime: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 检索命中的 chunk 行（fts/vec CTE + RRF 查询的 SELECT 形状）。
#[derive(Debug, sqlx::FromRow)]
pub struct ChunkHitRow {
    pub id: Uuid,
    pub document_id: Uuid,
    pub seq: i32,
    pub content: String,
    pub embed_failed: bool,
    pub score: f64,
    pub title: String,
}
