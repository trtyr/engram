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

/// 资产台账全量（0058；2026-09-22 上云补齐——资产是「我拥有的东西」的唯一事实源，须随包走）。
pub async fn export_assets(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(a) FROM assets a ORDER BY a.name")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 工作线关联全量（0058 二部：part_of / related）。
pub async fn export_project_links(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(l) FROM project_links l ORDER BY l.from_project, l.to_project, l.kind",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
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

/// 项目文件 + 版本历史（2026-09-22 上云核账补齐）：项目文件是真实产物内容（HTML/报告），
/// 派生重建不了——不随包走就是丢数据（本机 79 行 / 33 版本 vs 云机 0）。
pub async fn export_project_files(pool: &PgPool) -> StoreResult<(Vec<Value>, Vec<Value>)> {
    let files: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(f) FROM project_files f ORDER BY f.project_id, f.name")
            .fetch_all(pool)
            .await?;
    let versions: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(v) FROM project_file_versions v ORDER BY v.file_id, v.version",
    )
    .fetch_all(pool)
    .await?;
    Ok((files, versions))
}

/// atom↔entity 关联（圈子图的边；重建要重跑 LLM 抽取，随包走）。
pub async fn export_atom_entities(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(ae) FROM atom_entities ae ORDER BY ae.atom_id, ae.entity_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 技能版本历史（skill_revisions；当前内容在 skills/skill_files，这里补历史）。
pub async fn export_skill_revisions(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM skill_revisions r ORDER BY r.skill_id, r.rev")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// 工单/待办关联（同根因 link 成组的边）。
pub async fn export_todo_links(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(l) FROM todo_links l ORDER BY l.from_id, l.to_id, l.kind",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// wiki 源文档行（摄取台账的文档侧；分块挂在它下面）。
pub async fn export_wiki_documents(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(d) FROM wiki_documents d ORDER BY d.created_at")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// wiki 分块（剔除派生列 embedding/tsv——向量空缺由 re-embed 补，导入时落 embed_failed=true）。
pub async fn export_wiki_chunks(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(c) - 'embedding' - 'tsv' FROM wiki_chunks c ORDER BY c.document_id, c.seq",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// wiki 来源行（sha256 幂等键 + 原始路径 + 摄取状态）。
pub async fn export_wiki_sources(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(s) FROM wiki_sources s ORDER BY s.created_at")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// wiki 复核项（ingest 时 LLM 标的「建议建页/深度检索/需人判断」队列）。
pub async fn export_wiki_review_items(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM wiki_review_items r ORDER BY r.created_at")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// wiki 页间链接图（5007 行 / 本机；页面的 markdown 链接抽出物，是图谱页的数据源）。
pub async fn export_wiki_links(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(l) FROM wiki_links l ORDER BY l.from_slug, l.to_slug")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}
