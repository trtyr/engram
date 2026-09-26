//! 凭据域行类型（credentials：API Key / Token 等机密的一等台账，EN-234）。
//!
//! **安全设计**：值静态加密（KeyCipher），明文永不落库；列表/元数据 DTO 永不携带值——
//! 值只在按名取用（get）的响应里出现一次，且每次取用都留审计痕（credential_reads + 计数器）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// 凭据元数据（**不含值**——list/审计场景用）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CredentialMetaDto {
    pub id: Uuid,
    /// 按名取用的唯一键
    pub name: String,
    /// 敏感标记（内建；本域一切条目默认 sensitive）
    pub sensitive: bool,
    /// 用途说明（元信息，不含值）
    pub description: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 取用审计：最近一次 get
    pub last_read_at: Option<DateTime<Utc>>,
    /// 取用审计：累计 get 次数
    pub read_count: i64,
}

/// 取用审计流水行。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CredentialReadRow {
    pub id: Uuid,
    pub credential_id: Uuid,
    pub reader: String,
    pub read_at: DateTime<Utc>,
}

/// 解密后的取用结果（get 专用；值在此出现一次）。
#[derive(Debug, Clone, Serialize)]
pub struct CredentialValueDto {
    pub name: String,
    /// 直接可用的凭据值（只在 get 响应中出现）
    pub value: String,
    pub sensitive: bool,
    pub description: String,
    pub last_read_at: Option<DateTime<Utc>>,
    pub read_count: i64,
}
