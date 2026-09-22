//! `transfer` 的分域导入实现切片（2026-09-22 结构红线拆分：自 transfer.rs 纯搬移，零行为变化）。
//!
//! 编排外键序即调用序：memory → atom_entities → skills → skill_revisions →
//! wiki（库+页 → 五表）→ assets → projects(+files) → project_links →
//! todos/kv/promotions → todo_links。入口 `run_bundle` 只被 transfer.rs::import_bundle
//! （格式校验后）调用。

use super::*;

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

/// memory 域导入计数（六个子域）。
struct MemoryImported {
    sessions: DomainCount,
    atoms: DomainCount,
    scenarios: DomainCount,
    persona: DomainCount,
    entities: DomainCount,
    relations: DomainCount,
}

/// 包内 library_id → 目标库 id 的映射（多库迁移用）。
type LibMap = std::collections::HashMap<String, uuid::Uuid>;

/// 导入编排全量（外键序固定）。transfer.rs::import_bundle 格式校验后委托到这里。
pub(super) async fn run_bundle(pool: &PgPool, data: &Value) -> Result<Value> {
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
