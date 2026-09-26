//! 资产域服务：台账 CRUD（主机 / 云实例 / 域名 / 账号 / 设备）。
//!
//! **设计**（2026-09-21 用户拍板，见《项目与资产模型 · README》§2）：
//! 资产 = 「我拥有的、可以被操作的东西」——没有「收尾」、身份唯一、会被多个项目引用、
//! 要留变更痕。这与工作线（projects）的治理规则不同，故按治理规则独立成域；项目侧只**引用**
//! （`project_locations.asset_id`），不复制身份字段（唯一事实源铁律）。
//!
//! 别名（aliases）是「同一台机器多种写法」的收敛器：`trtyr-mac` / `demotestdeMacBook-Air.local`
//! 这类历史写法都挂在同一条目下，引用匹配也认别名。

use engram_storage::models::asset::AssetRevisionRow;
use engram_storage::repo::asset as repo;
use engram_storage::{PgPool, StoreError};
use serde::Serialize;
use uuid::Uuid;

/// 资产类型值域事实源 —— 与迁移 0058 的 CHECK 一一对应（改值域 = 改此处 + 一条迁移）。
pub const ASSET_KINDS: &[(&str, &str)] = &[
    ("host", "主机"),
    ("cloud", "云实例"),
    ("domain", "域名"),
    ("account", "账号"),
    ("device", "设备"),
    ("other", "其他"),
];

/// 支持的类型值域文案（错误文案共用同一事实源，如 `host/cloud/domain/account/device/other`）。
pub fn supported_kinds() -> String {
    ASSET_KINDS
        .iter()
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join("/")
}

/// 类型显示名（Web 与 MCP 共用）。
pub fn kind_label(kind: &str) -> String {
    ASSET_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| (*l).to_string())
        .unwrap_or_else(|| kind.to_string())
}

pub fn is_valid_kind(kind: &str) -> bool {
    ASSET_KINDS.iter().any(|(k, _)| *k == kind)
}

/// 类型模板项（前端下拉用）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AssetKindDto {
    pub kind: String,
    pub label: String,
}

/// 资产域错误（api / mcp 层各自转译）。
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for AssetError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(_) => {
                AssetError::Conflict("唯一约束冲突（同名资产已存在）".into())
            }
            StoreError::Sql(e) => AssetError::Storage(e.to_string()),
        }
    }
}

// ---------- DTO ----------

pub use engram_storage::models::asset::AssetDto;
pub use engram_storage::repo::asset::AssetUsageRow;

/// 资产详情 = 本体 + 被哪些项目位置引用（反查；「关系」区的数据源）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AssetDetailDto {
    #[serde(flatten)]
    pub asset: AssetDto,
    pub used_by: Vec<AssetUsageRow>,
    /// 运行手册正文（Markdown；主动记录——硬件/网络/服务/端口/变更/踩坑）。
    pub runbook_md: String,
}

// ---------- Service ----------

pub struct AssetService {
    pool: PgPool,
}

impl AssetService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 类型模板（前端下拉 / AI 选类型用）。
    pub fn list_kinds() -> Vec<AssetKindDto> {
        ASSET_KINDS
            .iter()
            .map(|(k, l)| AssetKindDto {
                kind: (*k).to_string(),
                label: (*l).to_string(),
            })
            .collect()
    }

    pub async fn list(
        &self,
        kind: Option<&str>,
        q: Option<&str>,
    ) -> Result<Vec<AssetDto>, AssetError> {
        if let Some(k) = kind.filter(|k| !is_valid_kind(k)) {
            return Err(AssetError::BadRequest(format!(
                "未知资产类型: {k}——支持 {}",
                supported_kinds()
            )));
        }
        let q = q.map(str::trim).filter(|s| !s.is_empty());
        Ok(repo::list_assets(&self.pool, kind, q).await?)
    }

    /// 详情（本体 + 被引用位置 + 运行手册）。
    pub async fn get(&self, id: Uuid) -> Result<AssetDetailDto, AssetError> {
        let asset = repo::get_asset(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("资产 {id} 不存在——先 list 定位")))?;
        let used_by = repo::projects_using(&self.pool, id).await?;
        let runbook_md = repo::get_runbook(&self.pool, id).await?.unwrap_or_default();
        Ok(AssetDetailDto {
            asset,
            used_by,
            runbook_md,
        })
    }

    /// 运行手册：读某资产的 Markdown 正文。
    pub async fn runbook(&self, id: Uuid) -> Result<String, AssetError> {
        repo::get_runbook(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("资产 {id} 不存在——先 list 定位")))
    }

    /// 运行手册：保存（同事务旧文进修订史——错改可回滚，变更可追溯）。
    pub async fn save_runbook(&self, id: Uuid, md: &str, editor: &str) -> Result<(), AssetError> {
        if !repo::save_runbook(&self.pool, id, md, editor).await? {
            return Err(AssetError::NotFound(format!(
                "资产 {id} 不存在——先 list 定位"
            )));
        }
        Ok(())
    }

    /// 运行手册：修订史清单（新→旧；old_runbook_md = 该次保存前的正文）。
    pub async fn runbook_versions(&self, id: Uuid) -> Result<Vec<AssetRevisionRow>, AssetError> {
        // 资产存在性校验（空清单与不存在要可区分）
        repo::get_runbook(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("资产 {id} 不存在——先 list 定位")))?;
        repo::runbook_versions(&self.pool, id)
            .await
            .map_err(Into::into)
    }

    /// 运行手册：回滚到某修订（把该版旧文存回；当前正文先入史——回滚本身也留痕）。
    pub async fn restore_runbook(
        &self,
        id: Uuid,
        version_id: Uuid,
        editor: &str,
    ) -> Result<(), AssetError> {
        let rev = repo::get_runbook_version(&self.pool, version_id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("修订 {version_id} 不存在")))?;
        if rev.asset_id != id {
            return Err(AssetError::BadRequest("修订不属于该资产".into()));
        }
        self.save_runbook(id, &rev.old_runbook_md, editor).await
    }

    /// 按名称或别名解析（项目侧引用的解析入口；大小写不敏感）。
    pub async fn get_by_name_or_alias(&self, key: &str) -> Result<AssetDto, AssetError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(AssetError::BadRequest("名称/别名不能为空".into()));
        }
        let id = repo::find_by_name_or_alias(&self.pool, key, None)
            .await?
            .ok_or_else(|| {
                AssetError::NotFound(format!(
                    "没有名称或别名为「{key}」的资产——先 asset_list 看台账，或用 add 建档"
                ))
            })?;
        repo::get_asset(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("资产 {id} 不存在")))
    }

    /// 新建资产（名称与别名共享同一个命名空间：都不许与既有条目撞）。
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        kind: &str,
        name: &str,
        aliases: &[String],
        ip: &str,
        os: &str,
        note: &str,
    ) -> Result<AssetDto, AssetError> {
        if !is_valid_kind(kind) {
            return Err(AssetError::BadRequest(format!(
                "未知资产类型: {kind}——支持 {}",
                supported_kinds()
            )));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(AssetError::BadRequest("资产名不能为空".into()));
        }
        let aliases = clean_aliases(aliases, name);
        // 先查后写：名称与每个别名都要独占（并发窗口由 UNIQUE 兜底）
        for key in std::iter::once(name).chain(aliases.iter().map(String::as_str)) {
            if let Some(holder) = repo::find_by_name_or_alias(&self.pool, key, None).await? {
                let holder_name = repo::get_asset(&self.pool, holder)
                    .await?
                    .map(|a| a.name)
                    .unwrap_or_else(|| holder.to_string());
                return Err(AssetError::Conflict(format!(
                    "「{key}」已被资产「{holder_name}」占用（名称与别名共享命名空间）——改用它做别名，或先改那条"
                )));
            }
        }
        let id = Uuid::now_v7();
        let n = repo::insert_asset(
            &self.pool,
            id,
            kind,
            name,
            &aliases,
            ip.trim(),
            os.trim(),
            note.trim(),
        )
        .await?;
        if n == 0 {
            return Err(AssetError::Conflict(format!("同名资产「{name}」已存在")));
        }
        repo::get_asset(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::Storage("插入后读回失败".into()))
    }

    /// 编辑资产（补丁式：不传的字段不动）。
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: Uuid,
        kind: Option<&str>,
        name: Option<&str>,
        aliases: Option<&[String]>,
        ip: Option<&str>,
        os: Option<&str>,
        note: Option<&str>,
        fields: Option<&serde_json::Value>,
    ) -> Result<AssetDto, AssetError> {
        let cur = repo::get_asset(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("资产 {id} 不存在——先 list 定位")))?;
        let kind = match kind {
            Some(k) => {
                if !is_valid_kind(k) {
                    return Err(AssetError::BadRequest(format!(
                        "未知资产类型: {k}——支持 {}",
                        supported_kinds()
                    )));
                }
                k.to_string()
            }
            None => cur.kind.clone(),
        };
        let name = match name {
            Some(n) => {
                let n = n.trim();
                if n.is_empty() {
                    return Err(AssetError::BadRequest("资产名不能为空".into()));
                }
                n.to_string()
            }
            None => cur.name.clone(),
        };
        let aliases = match aliases {
            Some(a) => clean_aliases(a, &name),
            None => cur.aliases.clone(),
        };
        // 占用预检（排除自身）
        for key in std::iter::once(&name).chain(aliases.iter()) {
            if let Some(holder) = repo::find_by_name_or_alias(&self.pool, key, Some(id)).await? {
                let holder_name = repo::get_asset(&self.pool, holder)
                    .await?
                    .map(|a| a.name)
                    .unwrap_or_else(|| holder.to_string());
                return Err(AssetError::Conflict(format!(
                    "「{key}」已被资产「{holder_name}」占用——换一个名字/别名"
                )));
            }
        }
        let n = repo::update_asset(
            &self.pool,
            id,
            &kind,
            &name,
            &aliases,
            ip.map(str::trim).unwrap_or(cur.ip.as_str()),
            os.map(str::trim).unwrap_or(cur.os.as_str()),
            note.map(str::trim).unwrap_or(cur.note.as_str()),
            fields,
        )
        .await?;
        if n == 0 {
            return Err(AssetError::NotFound(format!("资产 {id} 不存在")));
        }
        repo::get_asset(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::Storage("更新后读回失败".into()))
    }

    /// 删除资产：**被项目引用时拒绝**（唯一事实源不许静默断链；先解绑位置引用再删）。
    pub async fn delete(&self, id: Uuid) -> Result<bool, AssetError> {
        let cur = repo::get_asset(&self.pool, id)
            .await?
            .ok_or_else(|| AssetError::NotFound(format!("资产 {id} 不存在")))?;
        let used = repo::projects_using(&self.pool, id).await?;
        if !used.is_empty() {
            let names: Vec<String> = used
                .iter()
                .map(|u| {
                    if u.host.is_empty() {
                        u.project_name.clone()
                    } else {
                        format!("{}（{}）", u.project_name, u.host)
                    }
                })
                .collect();
            return Err(AssetError::Conflict(format!(
                "资产「{}」还被 {} 个项目位置引用：{}——先解绑（改那些位置或删位置行）再删；\
                 只是退役不去用，用 update 改 note 标注即可",
                cur.name,
                used.len(),
                names.join(" / ")
            )));
        }
        Ok(repo::delete_asset(&self.pool, id).await? > 0)
    }
}

/// 别名清洗：去空、去首尾空白、去掉与主名重复的项。（保留顺序，不自动去重大小写差异以外的项）
fn clean_aliases(aliases: &[String], name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in aliases {
        let a = a.trim();
        if a.is_empty() || a.eq_ignore_ascii_case(name) {
            continue;
        }
        if out.iter().any(|x: &String| x.eq_ignore_ascii_case(a)) {
            continue;
        }
        out.push(a.to_string());
    }
    out
}
