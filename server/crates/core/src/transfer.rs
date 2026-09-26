//! 迁移编排：全系统导出聚合、导入（冲突跳过，分域报告）、技能导入闭环。
//!
//! 迁移包格式（engram-transfer v1）：
//! `{format, version, exported_at, counts, memory:{sessions,atoms,scenarios,persona,entities,relations},
//!   wiki:{pages:[...]}, projects:{projects,locations,docs}}`
//!
//! 导入语义：冲突跳过（主键/slug/name 已存在即 skip）——迁移（空机全量进）与
//! 合并（两机并集）都可重复执行。派生列（embedding/tsv）不迁移，导入时重建
//! tsv；向量空缺用 memory reembed / wiki re-embed 补。codegraph 索引是派生数据
//! 不迁移（B 机对同源仓库重新建索引即可）。
//!
//! 分域导入的实现切片（计数器 / 各域助手 / 编排体）在 `transfer/import.rs`；
//! 本文件只留导出聚合、入口编排与技能导入闭环。

use engram_storage::repo::transfer as repo;

mod import;
mod sync;
use engram_storage::{PgPool, StoreError};
use serde_json::{Value, json};
pub use sync::{check_sync_target, pull_from, sync_transfer};

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for TransferError {
    fn from(e: StoreError) -> Self {
        TransferError::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TransferError>;

/// 全系统导出聚合（迁移包 v1）。
pub async fn export_bundle(pool: &PgPool) -> Result<Value> {
    let (sessions, atoms, scenarios, persona, entities, relations) =
        repo::export_memory(pool).await?;
    let wiki_libraries = repo::export_wiki_libraries(pool).await?;
    let wiki_pages = repo::export_wiki_pages(pool).await?;
    let (projects, locations, docs) = repo::export_projects(pool).await?;
    // 资产与工作线关联（0058；2026-09-22 上云补齐——资产是唯一事实源，不随包会丢）
    let assets = repo::export_assets(pool).await?;
    let project_links = repo::export_project_links(pool).await?;
    // 2026-09-22 上云核账补齐（第二轮）：真实内容但先前不在包里的 10 张表——项目文件 + 版本历史、
    // atom↔entity 边、技能历史、工单关联、wiki 文档/分块/来源/复核项/页间链接。
    // （codegraph 索引仍是派生数据不迁移：B 机对同源仓库重新建索引即可。）
    let (project_files, project_file_versions) = repo::export_project_files(pool).await?;
    let atom_entities = repo::export_atom_entities(pool).await?;
    let todo_links = repo::export_todo_links(pool).await?;
    let wiki_documents = repo::export_wiki_documents(pool).await?;
    let wiki_chunks = repo::export_wiki_chunks(pool).await?;
    let wiki_sources = repo::export_wiki_sources(pool).await?;
    let wiki_review_items = repo::export_wiki_review_items(pool).await?;
    let wiki_links = repo::export_wiki_links(pool).await?;
    let todos = repo::export_todos(pool).await?;
    let kv_entries = repo::export_kv_entries(pool).await?;
    let wiki_promotions = repo::export_wiki_promotions(pool).await?;

    Ok(json!({
        "format": "engram-transfer",
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "counts": {
            "sessions": sessions.len(), "atoms": atoms.len(),
            "scenarios": scenarios.len(), "persona": persona.len(),
            "entities": entities.len(), "relations": relations.len(),
            "wiki_libraries": wiki_libraries.len(),
    "wiki_pages": wiki_pages.len(),
            "projects": projects.len(), "locations": locations.len(), "docs": docs.len(),
            "assets": assets.len(), "project_links": project_links.len(),
            "project_files": project_files.len(),
            "project_file_versions": project_file_versions.len(),
            "atom_entities": atom_entities.len(),
            "todo_links": todo_links.len(), "wiki_documents": wiki_documents.len(),
            "wiki_chunks": wiki_chunks.len(), "wiki_sources": wiki_sources.len(),
            "wiki_review_items": wiki_review_items.len(), "wiki_links": wiki_links.len(),
            "todos": todos.len(), "kv_entries": kv_entries.len(),
            "wiki_promotions": wiki_promotions.len(),
        },
        "memory": {
            "sessions": sessions, "atoms": atoms, "scenarios": scenarios,
            "persona": persona, "entities": entities, "relations": relations,
            "atom_entities": atom_entities,
        },
        "wiki": {
            "libraries": wiki_libraries, "pages": wiki_pages,
            "documents": wiki_documents, "chunks": wiki_chunks,
            "sources": wiki_sources, "review_items": wiki_review_items,
            "links": wiki_links,
        },
        "projects": { "projects": projects, "locations": locations, "docs": docs },
        "project_files": project_files,
        "project_file_versions": project_file_versions,
        "assets": assets,
        "project_links": project_links,
        "todos": todos,
        "todo_links": todo_links,
        "kv_entries": kv_entries,
        "wiki_promotions": wiki_promotions,
    }))
}

/// 导入迁移包（冲突跳过）。返回分域 {imported, skipped} 报告。
pub async fn import_bundle(pool: &PgPool, data: &Value) -> Result<Value> {
    if data.get("format").and_then(|x| x.as_str()) != Some("engram-transfer") {
        return Err(TransferError::BadRequest(
            "不是 engram-transfer 迁移包——请用 /migrate/export 的导出文件".into(),
        ));
    }
    import::run_bundle(pool, data).await
}

// ---------- 远程拉取（A → B 一键迁移） ----------
//
// B 机管理员提供 A 机地址 + A 的 admin 密码：登录 A → 拉迁移包 → 落地本机。
// A 的密码只在本次请求内使用（登录换 ams_ token），不落库不缓存。

// ---------- 双向同步转发（2026-09-18 数据同步线：CLI sync 的服务端形态） ----------
//
// 浏览器跨域无法直调目标实例，由本机服务端转发：push = 本地 export → POST 目标 import；
// pull = GET 目标 export → 灌本地 import。目标凭证用 migrate scope key（Bearer 直连），
// admin 密码不过目标网络。目标非 loopback 强制 https（allow_insecure 逃生）。

/// 双向同步转发结果：源包分域计数 +（非 dry_run 时）目标导入报告。
pub struct SyncOutcome {
    pub direction: &'static str,
    pub source_counts: Value,
    pub dry_run: bool,
    pub import_report: Option<Value>,
}
