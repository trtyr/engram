//! 资产域行类型（assets：主机 / 云实例 / 域名 / 账号 / 设备的一等台账）。
//!
//! **唯一事实源**：资产身份只此一份（0058）——项目侧通过 `project_locations.asset_id`
//! 引用它，不复制身份字段（详见《项目与资产模型 · README》§2.4「一处权威 + 指针」）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AssetDto {
    pub id: Uuid,
    /// 资产类型：host 主机 / cloud 云实例 / domain 域名 / account 账号 / device 设备 / other 其他
    pub kind: String,
    /// 台账名（唯一）
    pub name: String,
    /// 别名（主机名 / ssh 别名 / 历史写法）——引用匹配与历史归一的依据
    pub aliases: Vec<String>,
    /// 规范地址（公网或组网 IP；会变，变化走 update）
    pub ip: String,
    pub os: String,
    pub note: String,
    #[schema(value_type = Object)]
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
