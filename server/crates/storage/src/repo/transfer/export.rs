//! `transfer` 的实现切片（架构治理 2026-09-21：自 transfer.rs 纯搬移，零行为变化）。

use super::*;

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

/// wiki_promotions 全量导出（0047）。
pub async fn export_wiki_promotions(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(w) FROM wiki_promotions w ORDER BY w.project_id, w.page_slug",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
