//! 迁移编排：全系统导出聚合、导入（冲突跳过，分域报告）、技能导入闭环。
//!
//! 迁移包格式（engram-transfer v1）：
//! `{format, version, exported_at, counts, memory:{sessions,atoms,scenarios,persona,entities,relations},
//!   skills:[{skill...,files:[...]}], wiki:{pages:[...]}, projects:{projects,locations,docs}}`
//!
//! 导入语义：冲突跳过（主键/slug/name 已存在即 skip）——迁移（空机全量进）与
//! 合并（两机并集）都可重复执行。派生列（embedding/tsv）不迁移，导入时重建
//! tsv；向量空缺用 memory reembed / wiki re-embed 补。codegraph 索引是派生数据
//! 不迁移（B 机对同源仓库重新建索引即可）。

use engram_storage::repo::transfer as repo;

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
    let skills = repo::export_skills_with_files(pool).await?;
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
    let skill_revisions = repo::export_skill_revisions(pool).await?;
    let todo_links = repo::export_todo_links(pool).await?;
    let wiki_documents = repo::export_wiki_documents(pool).await?;
    let wiki_chunks = repo::export_wiki_chunks(pool).await?;
    let wiki_sources = repo::export_wiki_sources(pool).await?;
    let wiki_review_items = repo::export_wiki_review_items(pool).await?;
    let wiki_links = repo::export_wiki_links(pool).await?;
    let todos = repo::export_todos(pool).await?;
    let kv_entries = repo::export_kv_entries(pool).await?;
    let wiki_promotions = repo::export_wiki_promotions(pool).await?;

    let skills_json: Vec<Value> = skills
        .into_iter()
        .map(|(mut skill, files)| {
            skill["files"] = json!(files);
            skill
        })
        .collect();

    Ok(json!({
        "format": "engram-transfer",
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "counts": {
            "sessions": sessions.len(), "atoms": atoms.len(),
            "scenarios": scenarios.len(), "persona": persona.len(),
            "entities": entities.len(), "relations": relations.len(),
            "skills": skills_json.len(), "wiki_libraries": wiki_libraries.len(),
    "wiki_pages": wiki_pages.len(),
            "projects": projects.len(), "locations": locations.len(), "docs": docs.len(),
            "assets": assets.len(), "project_links": project_links.len(),
            "project_files": project_files.len(),
            "project_file_versions": project_file_versions.len(),
            "atom_entities": atom_entities.len(), "skill_revisions": skill_revisions.len(),
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
        "skills": skills_json,
        "skill_revisions": skill_revisions,
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

/// 分域导入计数（imported, skipped）。
#[derive(Default)]
struct DomainCount {
    imported: usize,
    skipped: usize,
}

impl DomainCount {
    fn bump(imported: bool) -> Self {
        if imported {
            DomainCount {
                imported: 1,
                skipped: 0,
            }
        } else {
            DomainCount {
                imported: 0,
                skipped: 1,
            }
        }
    }
    fn merge(&mut self, other: DomainCount) {
        self.imported += other.imported;
        self.skipped += other.skipped;
    }
    fn to_json(&self) -> Value {
        json!({ "imported": self.imported, "skipped": self.skipped })
    }
}

fn each(v: Option<&Value>, key: &str) -> Vec<Value> {
    v.and_then(|d| d.get(key))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 导入迁移包（冲突跳过）。返回分域 {imported, skipped} 报告。
pub async fn import_bundle(pool: &PgPool, data: &Value) -> Result<Value> {
    if data.get("format").and_then(|x| x.as_str()) != Some("engram-transfer") {
        return Err(TransferError::BadRequest(
            "不是 engram-transfer 迁移包——请用 /migrate/export 的导出文件".into(),
        ));
    }

    let mem = import_memory_domain(pool, data).await?;
    // atom↔entity 边须在 atoms/entities 之后（外键 → 两表）
    let c_atom_entity = import_atom_entities_domain(pool, data).await?;
    let (c_skill, files_imported) = import_skills_domain(pool, data).await?;
    // 技能历史须在 skills 之后（外键 → skills）
    let c_skill_rev = import_skill_revisions_domain(pool, data).await?;
    let (lib_map, c_wikilib, c_wiki) = import_wiki_domain(pool, data).await?;
    // wiki 关联五表（文档/分块/来源/复核项/页间链接）复用库映射
    let (c_wdoc, c_wchunk, c_wsrc, c_wrev, c_wlink) =
        import_wiki_extras_domain(pool, data, &lib_map).await?;
    // 资产先于项目位置（project_locations.asset_id → assets，外键序）
    let c_asset = import_asset_domain(pool, data).await?;
    let (c_project, c_location, c_doc) = import_projects_domain(pool, data).await?;
    // 项目文件 + 版本历史（外键 → projects / project_files）
    let (c_pfile, c_pver) = import_project_files_domain(pool, data).await?;
    // 工作线关联：两端项目须已在库（外键 → projects）
    let c_link = import_project_links(pool, data).await?;
    let (t_imp, t_skip, kv_imp, kv_skip, p_imp, p_skip) = import_tail_domains(pool, data).await?;
    // 工单/待办关联须在 todos 之后（两端 id 都要在库）
    let c_todo_link = import_todo_links_domain(pool, data).await?;
    let (c_session, c_atom, c_scenario, c_persona, c_entity, c_relation) = (
        mem.sessions,
        mem.atoms,
        mem.scenarios,
        mem.persona,
        mem.entities,
        mem.relations,
    );

    Ok(json!({
        "format": "engram-transfer",
        "memory": {
            "sessions": c_session.to_json(), "atoms": c_atom.to_json(),
            "scenarios": c_scenario.to_json(), "persona": c_persona.to_json(),
            "entities": c_entity.to_json(), "relations": c_relation.to_json(),
            "atom_entities": c_atom_entity.to_json(),
        },
        "skills": {
            "skills": c_skill.to_json(), "files": { "imported": files_imported },
            "revisions": c_skill_rev.to_json(),
        },
        "wiki": {
            "libraries": c_wikilib.to_json(), "pages": c_wiki.to_json(),
            "documents": c_wdoc.to_json(), "chunks": c_wchunk.to_json(),
            "sources": c_wsrc.to_json(), "review_items": c_wrev.to_json(),
            "links": c_wlink.to_json(),
        },
        "projects": {
            "projects": c_project.to_json(), "locations": c_location.to_json(),
            "docs": c_doc.to_json(),
            "files": c_pfile.to_json(), "file_versions": c_pver.to_json(),
        },
        "assets": c_asset.to_json(),
        "project_links": c_link.to_json(),
        "todos": { "imported": t_imp, "skipped": t_skip },
        "todo_links": c_todo_link.to_json(),
        "kv_entries": { "imported": kv_imp, "skipped": kv_skip },
        "wiki_promotions": { "imported": p_imp, "skipped": p_skip },
        "note": "冲突（id/slug/name 已存在）按跳过处理；embedding 未迁移——\
                 memory 用 POST /memory/reembed、wiki 用文档域 re-embed 补齐",
    }))
}

/// 技能域导入闭环：吃 /skills/export（或迁移包 skills 数组）同构数据。
pub async fn import_skills_bundle(pool: &PgPool, items: &Value) -> Result<Value> {
    let list = items
        .as_array()
        .cloned()
        .or_else(|| items.get("skills").and_then(|x| x.as_array()).cloned())
        .ok_or_else(|| {
            TransferError::BadRequest(
                "格式不对：期望数组（/skills/export 的返回）或含 skills 字段的对象".into(),
            )
        })?;
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut files = 0usize;
    for s in list {
        let fs = s
            .get("files")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let (ok, fn_) = repo::import_skill(pool, &s, &fs).await?;
        if ok {
            imported += 1;
            files += fn_;
        } else {
            skipped += 1;
        }
    }
    Ok(json!({
        "imported": imported, "skipped": skipped, "files_imported": files,
        "note": "slug 已存在按跳过处理（不覆盖）——更新请走 skills_update",
    }))
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

/// memory 域导入计数（六个子域）。
struct MemoryImported {
    sessions: DomainCount,
    atoms: DomainCount,
    scenarios: DomainCount,
    persona: DomainCount,
    entities: DomainCount,
    relations: DomainCount,
}

/// memory 域导入：sessions → scenarios → atoms（外键序：scenarios 先于 atoms；自引用 FK 后置回填）
/// → persona → entities → relations。
async fn import_memory_domain(pool: &PgPool, data: &Value) -> Result<MemoryImported> {
    let mut c_session = DomainCount::default();
    for it in each(data.get("memory"), "sessions") {
        c_session.merge(DomainCount::bump(repo::import_session(pool, &it).await?));
    }
    // 外键序：scenarios 先于 atoms（atoms.scenario_id → scenarios）——顺序颠倒会在
    // 干净库导入时违反 atoms_scenario_id_fkey（公网加固 t9 往返演练抓出并修复）
    let mut c_scenario = DomainCount::default();
    for it in each(data.get("memory"), "scenarios") {
        c_scenario.merge(DomainCount::bump(repo::import_scenario(pool, &it).await?));
    }
    // superseded_by 是表内自引用 FK（atoms → atoms.id）——行序随机，插入期置 NULL，
    // 全量入库后统一回填（公网加固 t9 往返演练抓出）
    let mut c_atom = DomainCount::default();
    for it in each(data.get("memory"), "atoms") {
        let mut row = it.clone();
        if let Some(obj) = row.as_object_mut() {
            obj.insert("superseded_by".into(), serde_json::Value::Null);
        }
        c_atom.merge(DomainCount::bump(repo::import_atom(pool, &row).await?));
    }
    backfill_superseded_atoms(pool, data).await?;

    let mut c_persona = DomainCount::default();
    for it in each(data.get("memory"), "persona") {
        c_persona.merge(DomainCount::bump(repo::import_persona(pool, &it).await?));
    }
    let mut c_entity = DomainCount::default();
    for it in each(data.get("memory"), "entities") {
        c_entity.merge(DomainCount::bump(repo::import_entity(pool, &it).await?));
    }
    let mut c_relation = DomainCount::default();
    for it in each(data.get("memory"), "relations") {
        c_relation.merge(DomainCount::bump(
            repo::import_entity_relation(pool, &it).await?,
        ));
    }
    Ok(MemoryImported {
        sessions: c_session,
        atoms: c_atom,
        scenarios: c_scenario,
        persona: c_persona,
        entities: c_entity,
        relations: c_relation,
    })
}

/// 技能域导入（技能行 + 文件），返回 `(技能计数, 文件导入数)`。
async fn import_skills_domain(pool: &PgPool, data: &Value) -> Result<(DomainCount, usize)> {
    let mut c_skill = DomainCount::default();
    let mut files_imported = 0usize;
    for s in each(Some(data), "skills") {
        let files = s
            .get("files")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let (imported, files_n) = repo::import_skill(pool, &s, &files).await?;
        c_skill.merge(DomainCount::bump(imported));
        files_imported += files_n;
    }

    Ok((c_skill, files_imported))
}

/// 包内 library_id → 目标库 id 的映射（多库迁移用）。
type LibMap = std::collections::HashMap<String, uuid::Uuid>;

/// wiki 域导入（库 + 页面，库先于页——页面外键指向库）。
/// 返回 (库映射, 库计数, 页计数)——映射同时喂给 wiki 关联五表导入复用。
async fn import_wiki_domain(
    pool: &PgPool,
    data: &Value,
) -> Result<(LibMap, DomainCount, DomainCount)> {
    // wiki 多库迁移（2026-09-18 数据同步线补齐）：先导库行建「包内 library_id → 目标库 id」
    // 映射，页面按映射挂原库；旧包无 libraries 字段时页面 fallback main（v1 包兼容）。
    let mut lib_map: LibMap = std::collections::HashMap::new();
    let mut c_wikilib = DomainCount::default();
    for it in each(data.get("wiki"), "libraries") {
        let src_id = it
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if src_id.is_empty() {
            continue;
        }
        let (mapped, imported) = repo::import_wiki_library(pool, &it).await?;
        lib_map.insert(src_id, mapped);
        c_wikilib.merge(DomainCount::bump(imported));
    }
    let main_lib = repo::main_library_id(pool).await?;
    let mut c_wiki = DomainCount::default();
    for it in each(data.get("wiki"), "pages") {
        // EN-63：导入页 tsv 按 wiki 口径现算（slug+title+content、wiki 分词变体）
        let tsv_text = engram_search::tokenize::tsv_text_wiki(&format!(
            "{} {} {}",
            it.get("slug").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("title").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("content").and_then(|x| x.as_str()).unwrap_or(""),
        ));
        let target_lib = it
            .get("library_id")
            .and_then(|x| x.as_str())
            .and_then(|s| lib_map.get(s).copied())
            .unwrap_or(main_lib);
        c_wiki.merge(DomainCount::bump(
            repo::import_wiki_page(pool, &it, &tsv_text, target_lib).await?,
        ));
    }
    Ok((lib_map, c_wikilib, c_wiki))
}

/// wiki 关联五表导入（2026-09-22 上云核账补齐）：文档 → 分块 → 来源 → 复核项 → 页间链接。
/// 分块须在文档之后（FK document_id）；复核项须在来源之后（FK source_id）。
/// 分块的 tsv 与摄取侧同口径（`tsv_text`，非页面的 `page_tsv_text`）；embedding 不迁移。
async fn import_wiki_extras_domain(
    pool: &PgPool,
    data: &Value,
    lib_map: &LibMap,
) -> Result<(
    DomainCount,
    DomainCount,
    DomainCount,
    DomainCount,
    DomainCount,
)> {
    let main_lib = repo::main_library_id(pool).await?;
    let lib_of = |v: &Value| -> uuid::Uuid {
        v.get("library_id")
            .and_then(|x| x.as_str())
            .and_then(|s| lib_map.get(s).copied())
            .unwrap_or(main_lib)
    };

    let mut c_doc = DomainCount::default();
    for it in each(data.get("wiki"), "documents") {
        let lib = lib_of(&it);
        c_doc.merge(DomainCount::bump(
            repo::import_wiki_document(pool, &it, lib).await?,
        ));
    }
    let mut c_chunk = DomainCount::default();
    for it in each(data.get("wiki"), "chunks") {
        let lib = lib_of(&it);
        let tsv_text = engram_search::tokenize::tsv_text(
            it.get("content").and_then(|x| x.as_str()).unwrap_or(""),
        );
        c_chunk.merge(DomainCount::bump(
            repo::import_wiki_chunk(pool, &it, &tsv_text, lib).await?,
        ));
    }
    let mut c_source = DomainCount::default();
    for it in each(data.get("wiki"), "sources") {
        let lib = lib_of(&it);
        c_source.merge(DomainCount::bump(
            repo::import_wiki_source(pool, &it, lib).await?,
        ));
    }
    let mut c_review = DomainCount::default();
    for it in each(data.get("wiki"), "review_items") {
        let lib = lib_of(&it);
        c_review.merge(DomainCount::bump(
            repo::import_wiki_review_item(pool, &it, lib).await?,
        ));
    }
    let mut c_link = DomainCount::default();
    for it in each(data.get("wiki"), "links") {
        let lib = lib_of(&it);
        c_link.merge(DomainCount::bump(
            repo::import_wiki_link(pool, &it, lib).await?,
        ));
    }
    Ok((c_doc, c_chunk, c_source, c_review, c_link))
}

/// atom↔entity 边导入（2026-09-22 补齐；须在 atoms 与 entities 之后）。
async fn import_atom_entities_domain(pool: &PgPool, data: &Value) -> Result<DomainCount> {
    let mut c = DomainCount::default();
    for it in each(data.get("memory"), "atom_entities") {
        c.merge(DomainCount::bump(
            repo::import_atom_entity(pool, &it).await?,
        ));
    }
    Ok(c)
}

/// 技能版本历史导入（须在 skills 之后）。
async fn import_skill_revisions_domain(pool: &PgPool, data: &Value) -> Result<DomainCount> {
    let mut c = DomainCount::default();
    for it in each(Some(data), "skill_revisions") {
        c.merge(DomainCount::bump(
            repo::import_skill_revision(pool, &it).await?,
        ));
    }
    Ok(c)
}

/// 项目文件 + 版本历史导入（2026-09-22 补齐；须在 projects 之后，版本再在文件之后）。
async fn import_project_files_domain(
    pool: &PgPool,
    data: &Value,
) -> Result<(DomainCount, DomainCount)> {
    let mut c_file = DomainCount::default();
    for it in each(Some(data), "project_files") {
        c_file.merge(DomainCount::bump(
            repo::import_project_file(pool, &it).await?,
        ));
    }
    let mut c_ver = DomainCount::default();
    for it in each(Some(data), "project_file_versions") {
        c_ver.merge(DomainCount::bump(
            repo::import_project_file_version(pool, &it).await?,
        ));
    }
    Ok((c_file, c_ver))
}

/// 工单/待办关联导入（2026-09-22 补齐；须在 todos 之后——两端 id 都要在库）。
async fn import_todo_links_domain(pool: &PgPool, data: &Value) -> Result<DomainCount> {
    let mut c = DomainCount::default();
    for it in each(Some(data), "todo_links") {
        c.merge(DomainCount::bump(repo::import_todo_link(pool, &it).await?));
    }
    Ok(c)
}

/// 项目域导入（项目 → 位置 → 文档）。
async fn import_projects_domain(
    pool: &PgPool,
    data: &Value,
) -> Result<(DomainCount, DomainCount, DomainCount)> {
    let mut c_project = DomainCount::default();
    for it in each(data.get("projects"), "projects") {
        c_project.merge(DomainCount::bump(repo::import_project(pool, &it).await?));
    }
    let mut c_location = DomainCount::default();
    for it in each(data.get("projects"), "locations") {
        c_location.merge(DomainCount::bump(
            repo::import_project_location(pool, &it).await?,
        ));
    }
    let mut c_doc = DomainCount::default();
    for it in each(data.get("projects"), "docs") {
        c_doc.merge(DomainCount::bump(
            repo::import_project_doc(pool, &it).await?,
        ));
    }
    Ok((c_project, c_location, c_doc))
}

/// 资产域导入（0058；2026-09-22 上云补齐）——必须在项目位置之前（外键 → assets）。
async fn import_asset_domain(pool: &PgPool, data: &Value) -> Result<DomainCount> {
    let mut c = DomainCount::default();
    for it in each(Some(data), "assets") {
        c.merge(DomainCount::bump(repo::import_asset(pool, &it).await?));
    }
    Ok(c)
}

/// 工作线关联导入（0058 二部）——两端项目须已在库（外键 → projects）。
async fn import_project_links(pool: &PgPool, data: &Value) -> Result<DomainCount> {
    let mut c = DomainCount::default();
    for it in each(Some(data), "project_links") {
        c.merge(DomainCount::bump(
            repo::import_project_link(pool, &it).await?,
        ));
    }
    Ok(c)
}

/// todos / kv / promotions 三域导入（t9 补齐 v1 覆盖缺口）。
async fn import_tail_domains(
    pool: &PgPool,
    data: &Value,
) -> Result<(usize, usize, usize, usize, usize, usize)> {
    // todos / kv / promotions 域（t9 补齐 v1 覆盖缺口；promotions 的 library 映射目标 main 库）
    let todo_items = data
        .get("todos")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let (t_imp, t_skip) = repo::import_todos(pool, &todo_items).await?;

    let kv_items = data
        .get("kv_entries")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let (kv_imp, kv_skip) = repo::import_kv_entries(pool, &kv_items).await?;

    let promo_items = data
        .get("wiki_promotions")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let (p_imp, p_skip) = repo::import_wiki_promotions(pool, &promo_items).await?;
    Ok((t_imp, t_skip, kv_imp, kv_skip, p_imp, p_skip))
}

/// atoms.superseded_by 自引用 FK 回填（插入期置 NULL，全量入库后统一回填）。
async fn backfill_superseded_atoms(pool: &PgPool, data: &Value) -> Result<()> {
    for it in each(data.get("memory"), "atoms") {
        let superseded_by = it
            .get("superseded_by")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if superseded_by.is_empty() {
            continue;
        }
        let (Ok(id), Ok(target)) = (
            uuid::Uuid::parse_str(it.get("id").and_then(|x| x.as_str()).unwrap_or("")),
            uuid::Uuid::parse_str(superseded_by),
        ) else {
            continue;
        };
        repo::backfill_atom_superseded_by(pool, id, target).await?;
    }
    Ok(())
}
