//! `transfer` 的实现切片（架构治理 2026-09-21：自 transfer.rs 纯搬移，零行为变化）。

use super::*;

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
    // asset_id 解析（2026-09-22 上云补齐）：先按导出包里的 id 认；目标库没这条 id 时
    // 退回按「台账名 / 别名」匹配（与 0058 存量清洗同规则）——两者都落空则留 NULL（不炸 FK）。
    let res = sqlx::query(
        "INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose, sort_order, asset_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, \
                 COALESCE( \
                   (SELECT a.id FROM assets a WHERE a.id = $9), \
                   (SELECT a.id FROM assets a WHERE $4 <> '' AND (a.name = $4 OR $4 = ANY (a.aliases)) LIMIT 1))) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "project_id"))
    .bind(str_of(v, "ip", ""))
    .bind(str_of(v, "host", ""))
    .bind(str_of(v, "os", ""))
    .bind(str_of(v, "path", ""))
    .bind(v.get("purpose").and_then(|x| x.as_str()))
    .bind(v.get("sort_order").and_then(|x| x.as_i64()).unwrap_or(0) as i32)
    .bind(id_of(v, "asset_id"))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 资产台账（0058；2026-09-22 上云补齐）。主键或台账名冲突都跳过（catch-all ON CONFLICT）。
pub async fn import_asset(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let aliases: Vec<String> = v
        .get("aliases")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let res = sqlx::query(
        "INSERT INTO assets (id, kind, name, aliases, ip, os, note, fields, created_at, updated_at) \
         VALUES ($1, $2, $3, $4::text[], $5, $6, $7, $8, $9, $10) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "kind", "other"))
    .bind(str_of(v, "name", ""))
    .bind(&aliases)
    .bind(str_of(v, "ip", ""))
    .bind(str_of(v, "os", ""))
    .bind(str_of(v, "note", ""))
    .bind(
        v.get("fields")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    )
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 工作线关联（0058 二部）：主键或 (from,to,kind) 重复均跳过。
pub async fn import_project_link(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO project_links (id, from_project, to_project, kind, note, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "from_project"))
    .bind(id_of(v, "to_project"))
    .bind(str_of(v, "kind", "related"))
    .bind(str_of(v, "note", ""))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
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

/// 项目文件（2026-09-22 上云核账补齐）：项目工作线里的真实产物内容（HTML/报告/设计稿）——
/// 派生不出来，必须随包。主键或 (project_id, name) 冲突均跳过（catch-all ON CONFLICT）。
pub async fn import_project_file(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO project_files (id, project_id, name, mime, content, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "project_id"))
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "mime", "text/plain"))
    .bind(str_of(v, "content", ""))
    .bind(v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 项目文件版本历史（外键 → project_files，须在其后导入）。
pub async fn import_project_file_version(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO project_file_versions (id, file_id, version, content, created_at) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "file_id"))
    .bind(v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(str_of(v, "content", ""))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 技能版本历史（外键 → skills，须在其后导入）。
pub async fn import_skill_revision(pool: &PgPool, v: &Value) -> StoreResult<bool> {
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
        "INSERT INTO skill_revisions (id, skill_id, rev, name, description, content, tags, origin, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::text[], $8, $9) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "skill_id"))
    .bind(v.get("rev").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "description", ""))
    .bind(str_of(v, "content", ""))
    .bind(&tags)
    .bind(str_of(v, "origin", "create"))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 工单/待办关联（两端 todo 须已在库；(from_id, to_id, kind) 冲突跳过）。
pub async fn import_todo_link(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO todo_links (id, from_id, to_id, kind, created_at) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "from_id"))
    .bind(id_of(v, "to_id"))
    .bind(str_of(v, "kind", "relates_to"))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}
