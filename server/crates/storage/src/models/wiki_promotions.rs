//! wiki_promotions：项目文档 → wiki 知识晋升登记（EN-59）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WikiPromotionDto {
    pub id: Uuid,
    pub library_id: Uuid,
    pub page_slug: String,
    pub project_id: Uuid,
    pub doc_id: Uuid,
    pub anchor: String,
    pub created_at: DateTime<Utc>,
}
