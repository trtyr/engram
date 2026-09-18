//! 迁移（transfer）仓储：各域导出读取 + 幂等导入 upsert。
//!
//! 导入语义：冲突跳过（主键/slug/name 已存在即 skip），保证迁移（空机全量进）
//! 与合并（两机并集）都可重复执行；逐条 imported/skipped 由编排层汇总。
//! 派生列不迁移：atoms/wiki 的 embedding 留空（reembed 可补），tsv 导入时按
//! content 重新生成；wiki_links 图边由织入流程重算。

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

// ---------- 导出读取 ----------

/// memory 域五表 + 实体关系（不含派生列 embedding/tsv/hit_count）。
pub async fn export_memory(
    pool: &PgPool,
) -> StoreResult<(
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
)> {
    let sessions: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(s) FROM raw_sessions s ORDER BY s.created_at")
            .fetch_all(pool)
            .await?;
    let atoms: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(a) - 'embedding' - 'tsv' - 'hit_count' FROM atoms a ORDER BY a.created_at",
    )
    .fetch_all(pool)
    .await?;
    let scenarios: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(s) - 'embedding' - 'hit_count' FROM scenarios s ORDER BY s.created_at",
    )
    .fetch_all(pool)
    .await?;
    let persona: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM persona_aspects p ORDER BY p.aspect, p.version",
    )
    .fetch_all(pool)
    .await?;
    let entities: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM entities e WHERE e.merged_into IS NULL ORDER BY e.created_at",
    )
    .fetch_all(pool)
    .await?;
    let relations: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM entity_relations r ORDER BY r.created_at")
            .fetch_all(pool)
            .await?;
    Ok((sessions, atoms, scenarios, persona, entities, relations))
}

/// wiki 页面（不含派生列 embedding/tsv）。
pub async fn export_wiki_pages(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) - 'embedding' - 'tsv' FROM wiki_pages p ORDER BY p.slug",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// wiki 库行（2026-09-18 数据同步线：多库迁移补齐——页面保留原库归属的前提）。
pub async fn export_wiki_libraries(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(l) FROM wiki_libraries l ORDER BY l.created_at")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// 项目域三表全量：(projects, locations, docs)。
pub async fn export_projects(pool: &PgPool) -> StoreResult<(Vec<Value>, Vec<Value>, Vec<Value>)> {
    let projects: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(p) FROM projects p ORDER BY p.created_at")
            .fetch_all(pool)
            .await?;
    let locations: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(l) FROM project_locations l ORDER BY l.project_id, l.sort_order",
    )
    .fetch_all(pool)
    .await?;
    let docs: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(d) FROM project_docs d ORDER BY d.project_id, d.category",
    )
    .fetch_all(pool)
    .await?;
    Ok((projects, locations, docs))
}

/// 待办全量（0035）。
pub async fn export_todos(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(t) FROM todos t ORDER BY (t.status = 'open') DESC, t.updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 导入待办（id 冲突跳过）。返回 (imported, skipped)。
pub async fn import_todos(pool: &PgPool, items: &[Value]) -> StoreResult<(usize, usize)> {
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for v in items {
        let res = sqlx::query(
            "INSERT INTO todos (id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at, kind, severity, symptom, reproduce, acceptance, resolution, resolved_at) \
             VALUES ($1, $2, $3, $4, $5, $6::text[], $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) ON CONFLICT (id) DO NOTHING",
        )
        .bind(
            v.get("id")
                .and_then(|x| x.as_str())
                .and_then(|s| Uuid::parse_str(s).ok()),
        )
        .bind(v.get("title").and_then(|x| x.as_str()).unwrap_or(""))
        .bind(v.get("body").and_then(|x| x.as_str()).unwrap_or(""))
        .bind(v.get("status").and_then(|x| x.as_str()).unwrap_or("open"))
        .bind(v.get("priority").and_then(|x| x.as_str()).unwrap_or("normal"))
        .bind(
            v.get("tags")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>())
                .unwrap_or_default(),
        )
        .bind(
            v.get("due_at")
                .and_then(|x| x.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))),
        )
        .bind(v.get("project_hint").and_then(|x| x.as_str()))
        .bind(
            v.get("done_at")
                .and_then(|x| x.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))),
        )
        .bind(
            v.get("created_at")
                .and_then(|x| x.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))),
        )
        .bind(
            v.get("updated_at")
                .and_then(|x| x.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))),
        )
        // 工单结构化列（t9 往返演练补齐——缺失会令 kind 落默认 todo，ticket 状态违反 todos_status_check）
        .bind(str_of(v, "kind", "todo"))
        .bind(v.get("severity").and_then(|x| x.as_str()))
        .bind(str_of(v, "symptom", ""))
        .bind(str_of(v, "reproduce", ""))
        .bind(str_of(v, "acceptance", ""))
        .bind(str_of(v, "resolution", ""))
        .bind(
            v.get("resolved_at")
                .and_then(|x| x.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))),
        )
        .execute(pool)
        .await?;
        if res.rows_affected() > 0 {
            imported += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((imported, skipped))
}

/// 技能全量：[(skill 行, files 行)]。
pub async fn export_skills_with_files(pool: &PgPool) -> StoreResult<Vec<(Value, Vec<Value>)>> {
    let skills: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(s) FROM skills s ORDER BY s.slug")
        .fetch_all(pool)
        .await?;
    let files: Vec<(String, Value)> = sqlx::query_as(
        "SELECT s.slug, to_jsonb(f) FROM skill_files f JOIN skills s ON s.id = f.skill_id ORDER BY s.slug, f.path",
    )
    .fetch_all(pool)
    .await?;
    let mut by_slug: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for (slug, f) in files {
        by_slug.entry(slug).or_default().push(f);
    }
    Ok(skills
        .into_iter()
        .map(|s| {
            let slug = s
                .get("slug")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let files = by_slug.remove(&slug).unwrap_or_default();
            (s, files)
        })
        .collect())
}

// ---------- 导入（幂等，冲突跳过） ----------

fn ts(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    v.get(key).and_then(|x| x.as_str()).and_then(|s| {
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    })
}

fn id_of(v: &Value, key: &str) -> Option<Uuid> {
    v.get(key)
        .and_then(|x| x.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn str_of(v: &Value, key: &str, default: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or(default)
        .to_string()
}

pub async fn import_session(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO raw_sessions (id, agent, content, distill_status, sensitive, created_at, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "agent", "unknown"))
    .bind(v.get("content").cloned().unwrap_or(serde_json::json!([])))
    .bind(str_of(v, "distill_status", "pending"))
    .bind(v.get("sensitive").and_then(|x| x.as_bool()).unwrap_or(false))
    .bind(ts(v, "created_at"))
    .bind(v.get("metadata").cloned().unwrap_or(serde_json::json!({})))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_atom(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let content = str_of(v, "content", "");
    let res = sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, superseded_by, needs_review, sensitive, scenario_id, occurred_at, valid_until, source_refs, tsv, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, to_tsvector('simple', $3), $13, $14) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "kind", "fact"))
    .bind(&content)
    .bind(v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.9) as f32)
    .bind(str_of(v, "status", "active"))
    .bind(id_of(v, "superseded_by"))
    .bind(v.get("needs_review").and_then(|x| x.as_bool()).unwrap_or(false))
    .bind(v.get("sensitive").and_then(|x| x.as_bool()).unwrap_or(false))
    .bind(id_of(v, "scenario_id"))
    .bind(ts(v, "occurred_at"))
    .bind(ts(v, "valid_until"))
    .bind(v.get("source_refs").cloned().unwrap_or(serde_json::json!([])))
    .bind(ts(v, "created_at"))
    .bind(ts(v, "updated_at"))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 回填 atoms 自引用 superseded_by（导入期置 NULL，全量入库后统一回填——t9 往返演练抓出）。
pub async fn backfill_atom_superseded_by(
    pool: &PgPool,
    id: uuid::Uuid,
    superseded_by: uuid::Uuid,
) -> StoreResult<()> {
    sqlx::query("UPDATE atoms SET superseded_by = $2 WHERE id = $1")
        .bind(id)
        .bind(superseded_by)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn import_scenario(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "topic", ""))
    .bind(str_of(v, "summary", ""))
    .bind(str_of(v, "body", ""))
    .bind(v.get("atom_refs").cloned().unwrap_or(serde_json::json!([])))
    .bind(v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_persona(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, manually_edited, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "aspect", ""))
    .bind(str_of(v, "content", ""))
    .bind(v.get("evidence_refs").cloned().unwrap_or(serde_json::json!([])))
    .bind(v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(v.get("prompt_version").and_then(|x| x.as_str()))
    .bind(v.get("manually_edited").and_then(|x| x.as_bool()).unwrap_or(false))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_entity(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO entities (id, name, kind, summary, manually_edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "kind", "topic"))
    .bind(str_of(v, "summary", ""))
    .bind(v.get("manually_edited").and_then(|x| x.as_bool()))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_entity_relation(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "from_id"))
    .bind(id_of(v, "to_id"))
    .bind(str_of(v, "rel_type", "related_to"))
    .bind(v.get("weight").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(str_of(v, "source", "manual"))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 技能：slug 冲突跳过；新插入时随行导入附属文件。返回 (skill_imported, files_imported)。
/// 二态字段（0038）随行：script 型只导元数据（content 空、files 无行）——指针与来源照收，
/// 导入端标记待本地就位（local_path 在新机器可能失效，get 时现读校验）。
pub async fn import_skill(pool: &PgPool, v: &Value, files: &[Value]) -> StoreResult<(bool, usize)> {
    let tags: Vec<String> = v
        .get("tags")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let kind = {
        let k = str_of(v, "kind", "text");
        if k == "script" { "script" } else { "text" }
    };
    let local_path: Option<String> = v
        .get("local_path")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    // script 型必须有指针（源导出保证；防御：缺失则降级 text，避免落地即违反 CHECK）
    let kind = if kind == "script" && local_path.is_none() {
        "text"
    } else {
        kind
    };
    let repo_url: Option<String> = v
        .get("repo_url")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let res = sqlx::query(
        "INSERT INTO skills (id, slug, name, description, content, tags, enabled, source, kind, origin, local_path, repo_url, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'import', $8, $9, $10, $11, $12, $13) ON CONFLICT (slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "slug", ""))
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "description", ""))
    .bind(str_of(v, "content", ""))
    .bind(&tags)
    .bind(v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true))
    .bind(kind)
    .bind(str_of(v, "origin", "self"))
    .bind(local_path)
    .bind(repo_url)
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Ok((false, 0));
    }
    // script 型不导入附属文件（真身在导入端本地，系统只存指针）
    if kind == "script" {
        return Ok((true, 0));
    }
    let skill_id: Uuid = sqlx::query_scalar("SELECT id FROM skills WHERE slug = $1")
        .bind(str_of(v, "slug", ""))
        .fetch_one(pool)
        .await?;
    let mut imported_files = 0;
    for f in files {
        let path = str_of(f, "path", "");
        if path.is_empty() {
            continue;
        }
        let r = sqlx::query(
            "INSERT INTO skill_files (id, skill_id, path, content) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (skill_id, path) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(skill_id)
        .bind(&path)
        .bind(str_of(f, "content", ""))
        .execute(pool)
        .await?;
        imported_files += r.rows_affected() as usize;
    }
    Ok((true, imported_files))
}

/// 导入 wiki 库行（按 slug 幂等），返回 (目标库 id, 是否新插入)——同名库已存在时映射到现有 id。
pub async fn import_wiki_library(pool: &PgPool, v: &Value) -> StoreResult<(Uuid, bool)> {
    let slug = str_of(v, "slug", "");
    let res = sqlx::query(
        "INSERT INTO wiki_libraries (id, slug, name) VALUES ($1, $2, $3) \
         ON CONFLICT (slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(&slug)
    .bind(str_of(v, "name", ""))
    .execute(pool)
    .await?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = $1")
        .bind(&slug)
        .fetch_one(pool)
        .await?;
    Ok((id, res.rows_affected() > 0))
}

/// main 库 id（旧迁移包无 wiki_libraries 域时，页面 fallback 落主库用）。
pub async fn main_library_id(pool: &PgPool) -> StoreResult<Uuid> {
    let id: Uuid = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
        .fetch_one(pool)
        .await?;
    Ok(id)
}

pub async fn import_wiki_page(
    pool: &PgPool,
    v: &Value,
    tsv_text: &str,
    target_lib: Uuid,
) -> StoreResult<bool> {
    let content = str_of(v, "content", "");
    // 多库迁移（2026-09-18 数据同步线补齐）：library 由调用方按 slug 映射传入，
    // 页面保留原库归属；旧包无 wiki_libraries 域时调用方 fallback main（v1 兼容）。
    // tsv 由调用方按 wiki 口径（slug+title+content、wiki 分词变体）算好传入——storage 不依赖分词器；
    // 系统页（index/log/overview）写 NULL（结构页不参与 FTS，EN-63 audit 回归教训）
    let res = sqlx::query(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, folder, tsv, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                 CASE WHEN $5 IN ('index','log','overview') THEN NULL ELSE to_tsvector('simple', $13) END, $11, $12) \
         ON CONFLICT (library_id, slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(target_lib)
    .bind(str_of(v, "slug", ""))
    .bind(str_of(v, "title", ""))
    .bind(str_of(v, "page_type", "concept"))
    .bind(&content)
    .bind(v.get("frontmatter").cloned().unwrap_or(serde_json::json!({})))
    .bind(str_of(v, "origin", "llm"))
    .bind(v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(str_of(v, "folder", ""))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .bind(tsv_text)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_project(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO projects (id, name, type, status, description, categories) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "type", "dev"))
    .bind(str_of(v, "status", "active"))
    .bind(v.get("description").and_then(|x| x.as_str()))
    .bind(
        v.get("categories")
            .cloned()
            .unwrap_or(serde_json::json!([])),
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_project_location(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose, sort_order) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "project_id"))
    .bind(str_of(v, "ip", ""))
    .bind(str_of(v, "host", ""))
    .bind(str_of(v, "os", ""))
    .bind(str_of(v, "path", ""))
    .bind(v.get("purpose").and_then(|x| x.as_str()))
    .bind(v.get("sort_order").and_then(|x| x.as_i64()).unwrap_or(0) as i32)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_project_doc(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO project_docs (id, project_id, category, folder, title, content) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "project_id"))
    .bind(str_of(v, "category", "规划"))
    .bind(str_of(v, "folder", ""))
    .bind(str_of(v, "title", ""))
    .bind(str_of(v, "content", ""))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

// ---------- 待办域（0035） ----------

/// 待办导出行。
pub type TodoExportRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Vec<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    DateTime<Utc>,
);

// ---------- KV / wiki_promotions 域（公网加固 t9 往返演练补齐 v1 覆盖缺口） ----------

/// KV 全量导出（0042；tsv 为生成列不迁移）。
pub async fn export_kv_entries(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(k) FROM kv_entries k ORDER BY k.key")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// KV 导入（key 唯一冲突跳过；tsv 由生成列自算）。
pub async fn import_kv_entries(pool: &PgPool, items: &[Value]) -> StoreResult<(usize, usize)> {
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for v in items {
        let tags: Vec<String> = v
            .get("tags")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let res = sqlx::query(
            "INSERT INTO kv_entries (id, key, value, context, tags, source, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (key) DO NOTHING",
        )
        .bind(v.get("id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(str_of(v, "key", ""))
        .bind(str_of(v, "value", ""))
        .bind(str_of(v, "context", ""))
        .bind(tags)
        .bind(str_of(v, "source", "user_stated"))
        .bind(ts(v, "created_at"))
        .bind(ts(v, "updated_at"))
        .execute(pool)
        .await?;
        if res.rows_affected() > 0 {
            imported += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((imported, skipped))
}

/// wiki_promotions 全量导出（0047）。
pub async fn export_wiki_promotions(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(w) FROM wiki_promotions w ORDER BY w.project_id, w.page_slug",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// promotions 导入（UNIQUE(project_id, doc_id, page_slug) 冲突跳过）。
/// library_id 统一映射目标库 main 库——多库 promotions 随 wiki 多库缺口记欠账（t9）。
pub async fn import_wiki_promotions(pool: &PgPool, items: &[Value]) -> StoreResult<(usize, usize)> {
    let Some(target_library_id): Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
            .fetch_optional(pool)
            .await?
    else {
        return Ok((0, items.len())); // 无 main 库（不该发生，0046 幂等保证）
    };
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for v in items {
        let res = sqlx::query(
            "INSERT INTO wiki_promotions (id, library_id, page_slug, project_id, doc_id, anchor, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (project_id, doc_id, page_slug) DO NOTHING",
        )
        .bind(v.get("id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(target_library_id)
        .bind(str_of(v, "page_slug", ""))
        .bind(v.get("project_id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(v.get("doc_id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(str_of(v, "anchor", ""))
        .bind(ts(v, "created_at"))
        .execute(pool)
        .await?;
        if res.rows_affected() > 0 {
            imported += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((imported, skipped))
}
