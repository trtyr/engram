//! 技能域仓储：skills / skill_revisions 两表读写。
//!
//! 事务说明：create / update / restore 都涉及「版本快照 + 本体变更」两步，
//! 事务整体封装在 `*_tx` 函数内（快照是否先行的语义判断留在服务层——
//! update 以 `Option<&SkillSnapshot>` 表达：None = enabled-only 变更不留版本）。

use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::skills::{SkillDto, SkillRevisionDto, SkillSummaryDto};

/// 版本快照保留上限（防膨胀；更老的自动淘汰）。
pub const MAX_REVISIONS: i32 = 50;

const SUMMARY_COLS: &str = "id, slug, name, description, tags, enabled, source, length(content)::bigint AS content_chars, kind, origin, local_path, repo_url, created_at, updated_at";
const FULL_COLS: &str = "id, slug, name, description, content, tags, enabled, source, kind, origin, local_path, repo_url, created_at, updated_at";
const REV_COLS: &str = "id, skill_id, rev, name, description, content, tags, origin, created_at";

/// 新建参数束（create_skill_tx 入参收拢）。
/// kind/origin/local_path/repo_url：二态存储（0038）——script 型带 local_path 指针；
/// snapshot=false（script 型）时不产 create 快照（无入库正文可快照）。
#[derive(Debug, Clone, Copy)]
pub struct NewSkillRow<'a> {
    pub id: Uuid,
    pub slug: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub content: &'a str,
    pub tags: &'a [String],
    pub enabled: bool,
    pub source: &'a str,
    pub kind: &'a str,
    pub origin: &'a str,
    pub local_path: Option<&'a str>,
    pub repo_url: Option<&'a str>,
    pub snapshot: bool,
}

/// 版本快照内容束（insert_revision_tx 入参收拢）。
#[derive(Debug, Clone, Copy)]
pub struct SkillSnapshot<'a> {
    pub skill_id: Uuid,
    pub name: &'a str,
    pub description: &'a str,
    pub content: &'a str,
    pub tags: &'a [String],
    pub origin: &'a str,
}

/// 更新的可选语义字段（None = 不动；COALESCE 语义）。
/// origin/repo_url/local_path 为全量写（服务层先读现状算好终值，与 kind 约束一致性由服务层保证）。
#[derive(Debug, Clone, Copy)]
pub struct SkillPatchData<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub content: Option<&'a str>,
    pub tags: &'a Option<Vec<String>>,
    pub enabled: Option<bool>,
    pub origin: &'a str,
    pub repo_url: Option<&'a str>,
    pub local_path: Option<&'a str>,
}

/// 事务内写一条版本快照 + 淘汰超限旧版。
async fn insert_revision_tx(
    conn: &mut sqlx::PgConnection,
    snap: &SkillSnapshot<'_>,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO skill_revisions (id, skill_id, rev, name, description, content, tags, origin) \
         VALUES ($1, $2, (SELECT COALESCE(MAX(rev), 0) + 1 FROM skill_revisions WHERE skill_id = $2), \
                 $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(snap.skill_id)
    .bind(snap.name)
    .bind(snap.description)
    .bind(snap.content)
    .bind(snap.tags)
    .bind(snap.origin)
    .execute(&mut *conn)
    .await?;
    prune_old_revisions(conn, snap.skill_id).await?;
    Ok(())
}

/// 淘汰超限旧版（保留最近 MAX_REVISIONS 版）。
async fn prune_old_revisions(conn: &mut sqlx::PgConnection, skill_id: Uuid) -> StoreResult<()> {
    sqlx::query(
        "DELETE FROM skill_revisions WHERE skill_id = $1 \
         AND rev <= (SELECT MAX(rev) FROM skill_revisions WHERE skill_id = $1) - $2",
    )
    .bind(skill_id)
    .bind(MAX_REVISIONS)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// 新建技能（事务：INSERT + create 快照 + 淘汰）。返回 skills 插入 rows_affected，
/// 0 = slug 冲突（ON CONFLICT DO NOTHING 未插入，事务回滚），由服务层转 Conflict。
pub async fn create_skill_tx(pool: &PgPool, row: NewSkillRow<'_>) -> StoreResult<u64> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "INSERT INTO skills (id, slug, name, description, content, tags, enabled, source, kind, origin, local_path, repo_url) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (slug) DO NOTHING",
    )
    .bind(row.id)
    .bind(row.slug)
    .bind(row.name)
    .bind(row.description)
    .bind(row.content)
    .bind(row.tags)
    .bind(row.enabled)
    .bind(row.source)
    .bind(row.kind)
    .bind(row.origin)
    .bind(row.local_path)
    .bind(row.repo_url)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(0);
    }
    if row.snapshot {
        insert_revision_tx(
            &mut tx,
            &SkillSnapshot {
                skill_id: row.id,
                name: row.name,
                description: row.description,
                content: row.content,
                tags: row.tags,
                origin: "create",
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(res.rows_affected())
}

/// 列表（摘要，不含正文）：q 搜 name/description，tag 过滤，enabled 过滤。
pub async fn list_skills(
    pool: &PgPool,
    pattern: Option<String>,
    tag_vec: Option<Vec<String>>,
    enabled: Option<bool>,
) -> StoreResult<Vec<SkillSummaryDto>> {
    // 三条件常驻 + 显式类型：避免条件拼接造成参数序号空洞（PG 推不出未引用参数的类型）
    let rows = sqlx::query_as::<_, SkillSummaryDto>(&format!(
        "SELECT {SUMMARY_COLS} FROM skills \
         WHERE ($1::text IS NULL OR name ILIKE $1 OR description ILIKE $1) \
         AND ($2::text[] IS NULL OR tags @> $2) \
         AND ($3::bool IS NULL OR enabled = $3) \
         ORDER BY updated_at DESC"
    ))
    .bind(pattern)
    .bind(tag_vec)
    .bind(enabled)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 slug 取详情（含正文）；不存在返回 None。
pub async fn get_skill(pool: &PgPool, slug: &str) -> StoreResult<Option<SkillDto>> {
    sqlx::query_as::<_, SkillDto>(&format!("SELECT {FULL_COLS} FROM skills WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 按技能名精确取（双寻址兜底：调用方记不住 slug 时用 name，R 报告 P1-11）。
pub async fn get_skill_by_name(pool: &PgPool, name: &str) -> StoreResult<Option<SkillDto>> {
    sqlx::query_as::<_, SkillDto>(&format!("SELECT {FULL_COLS} FROM skills WHERE name = $1"))
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 按 slug 删除，返回 rows_affected（0 = 不存在）。
pub async fn delete_skill(pool: &PgPool, slug: &str) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM skills WHERE slug = $1")
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 全量导出（含正文，按 slug 排序）。
pub async fn export_skills(pool: &PgPool) -> StoreResult<Vec<SkillDto>> {
    Ok(
        sqlx::query_as::<_, SkillDto>(&format!("SELECT {FULL_COLS} FROM skills ORDER BY slug"))
            .fetch_all(pool)
            .await?,
    )
}

/// slug 占用检查：命中返回占用者 id。
pub async fn exists_slug(pool: &PgPool, slug: &str) -> StoreResult<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM skills WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(id,)| id))
}

/// 某技能的全部版本快照（rev 降序）。
pub async fn list_revisions(pool: &PgPool, skill_id: Uuid) -> StoreResult<Vec<SkillRevisionDto>> {
    Ok(sqlx::query_as::<_, SkillRevisionDto>(&format!(
        "SELECT {REV_COLS} FROM skill_revisions WHERE skill_id = $1 ORDER BY rev DESC"
    ))
    .bind(skill_id)
    .fetch_all(pool)
    .await?)
}

/// 取单个版本快照（校验归属技能）。
pub async fn get_revision(
    pool: &PgPool,
    revision_id: Uuid,
    skill_id: Uuid,
) -> StoreResult<Option<SkillRevisionDto>> {
    sqlx::query_as::<_, SkillRevisionDto>(&format!(
        "SELECT {REV_COLS} FROM skill_revisions WHERE id = $1 AND skill_id = $2"
    ))
    .bind(revision_id)
    .bind(skill_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 语义字段更新（事务：可选先快照现状，再 COALESCE 落变更）。
/// snapshot = Some 时先写 origin=update 快照（语义变更）；None = enabled-only 不留版本。
/// 返回 UPDATE rows_affected（0 = 技能不存在，事务回滚），由服务层转 NotFound。
pub async fn update_skill_tx(
    pool: &PgPool,
    skill_id: Uuid,
    snapshot: Option<&SkillSnapshot<'_>>,
    patch: &SkillPatchData<'_>,
) -> StoreResult<u64> {
    let mut tx = pool.begin().await?;
    if let Some(snap) = snapshot {
        insert_revision_tx(&mut tx, snap).await?;
    }
    let res = sqlx::query(
        "UPDATE skills SET \
            name = COALESCE($2, name), \
            description = COALESCE($3, description), \
            content = COALESCE($4, content), \
            tags = COALESCE($5, tags), \
            enabled = COALESCE($6, enabled), \
            origin = $7, \
            repo_url = $8, \
            local_path = $9, \
            updated_at = now() \
         WHERE id = $1",
    )
    .bind(skill_id)
    .bind(patch.name)
    .bind(patch.description)
    .bind(patch.content)
    .bind(patch.tags)
    .bind(patch.enabled)
    .bind(patch.origin)
    .bind(patch.repo_url)
    .bind(patch.local_path)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(0);
    }
    tx.commit().await?;
    Ok(res.rows_affected())
}

/// 回滚到目标版本（事务：先快照现状 origin=restore，再把目标版本内容落回本体）。
pub async fn restore_revision_tx(
    pool: &PgPool,
    skill_id: Uuid,
    snapshot: &SkillSnapshot<'_>,
    restore: &SkillRevisionDto,
) -> StoreResult<()> {
    let mut tx = pool.begin().await?;
    insert_revision_tx(&mut tx, snapshot).await?;
    sqlx::query(
        "UPDATE skills SET name = $2, description = $3, content = $4, tags = $5, \
         updated_at = now() WHERE id = $1",
    )
    .bind(skill_id)
    .bind(&restore.name)
    .bind(&restore.description)
    .bind(&restore.content)
    .bind(&restore.tags)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------- 附属文件（folder 形态：scripts/ / references/ / assets/…） ----------
//
// skill = 文件夹：SKILL.md 本体在 skills.content；附属文件按相对路径寻址存本表。
// 云部署语义：文件是「内容」不是「文件系统位置」，MCP 按路径下发、客户端本地执行。

/// 文件索引行：(path, size 字节)。
pub async fn list_skill_files(pool: &PgPool, skill_id: Uuid) -> StoreResult<Vec<(String, i64)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT path, length(content)::bigint FROM skill_files WHERE skill_id = $1 ORDER BY path",
    )
    .bind(skill_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_skill_file(
    pool: &PgPool,
    skill_id: Uuid,
    path: &str,
) -> StoreResult<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT content FROM skill_files WHERE skill_id = $1 AND path = $2")
            .bind(skill_id)
            .bind(path)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(c,)| c))
}

/// upsert 单文件（同 path 幂等覆盖）。
pub async fn put_skill_file(
    pool: &PgPool,
    id: Uuid,
    skill_id: Uuid,
    path: &str,
    content: &str,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO skill_files (id, skill_id, path, content) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (skill_id, path) DO UPDATE SET content = $4, updated_at = now()",
    )
    .bind(id)
    .bind(skill_id)
    .bind(path)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_skill_file(pool: &PgPool, skill_id: Uuid, path: &str) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM skill_files WHERE skill_id = $1 AND path = $2")
        .bind(skill_id)
        .bind(path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 全量导出（数据主权）：(skill_id, path, content)。
pub async fn skill_files_all(pool: &PgPool) -> StoreResult<Vec<(Uuid, String, String)>> {
    let rows =
        sqlx::query_as("SELECT skill_id, path, content FROM skill_files ORDER BY skill_id, path")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// 文件变化触碰技能 updated_at（算技能活动；文件不出版本快照）。
pub async fn touch_skill(pool: &PgPool, skill_id: Uuid) -> StoreResult<()> {
    sqlx::query("UPDATE skills SET updated_at = now() WHERE id = $1")
        .bind(skill_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 单技能全部附属文件内容（bundle 导出用）：(path, content)。
pub async fn skill_file_contents(
    pool: &PgPool,
    skill_id: Uuid,
) -> StoreResult<Vec<(String, String)>> {
    let rows =
        sqlx::query_as("SELECT path, content FROM skill_files WHERE skill_id = $1 ORDER BY path")
            .bind(skill_id)
            .fetch_all(pool)
            .await?;
    Ok(rows)
}
