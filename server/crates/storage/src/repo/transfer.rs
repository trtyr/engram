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
    let res = sqlx::query(
        "INSERT INTO skills (id, slug, name, description, content, tags, enabled, source, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'import', $8, $9) ON CONFLICT (slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "slug", ""))
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "description", ""))
    .bind(str_of(v, "content", ""))
    .bind(&tags)
    .bind(v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Ok((false, 0));
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

pub async fn import_wiki_page(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let content = str_of(v, "content", "");
    let res = sqlx::query(
        "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, folder, tsv, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, to_tsvector('simple', $5), $10, $11) ON CONFLICT (slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
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
        "INSERT INTO project_docs (id, project_id, category, title, content) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "project_id"))
    .bind(str_of(v, "category", "规划"))
    .bind(str_of(v, "title", ""))
    .bind(str_of(v, "content", ""))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}
