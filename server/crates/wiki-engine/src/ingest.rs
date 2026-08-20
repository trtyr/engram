//! 两步 ingest job handlers：wiki_analyze → wiki_generate。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use sha2::Digest;
use uuid::Uuid;

use crate::markup::{extract_wikilinks, is_valid_slug};
use crate::prompts;

fn data_dir() -> std::path::PathBuf {
    std::env::var("AGENT_MEMORY_DATA_DIR")
        .unwrap_or_else(|_| "./data".into())
        .into()
}

fn wiki_sources_dir() -> std::path::PathBuf {
    data_dir().join("wiki-sources")
}

fn sha256_hex(b: &[u8]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(b);
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// 入队 ingest：source 内容（复用 knowledge 的解析产物文本或直接文本）。
/// sha 命中且已 ingest → 跳过（幂等）。
pub async fn enqueue_ingest(
    queue: &agent_memory_jobs::JobQueue,
    title: &str,
    text: &str,
) -> Result<(Uuid, bool), JobError> {
    let sha = sha256_hex(text.as_bytes());
    // 已有同 sha 且成功 ingest 的原料 → 幂等跳过
    let existing: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status FROM wiki_sources WHERE sha256 = $1")
            .bind(&sha)
            .fetch_optional(queue.pool())
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    if let Some((id, status)) = existing
        && status == "ready"
    {
        return Ok((id, true));
    }

    // 落不可变原料副本
    let dir = wiki_sources_dir();
    let _ = tokio::fs::create_dir_all(&dir).await;
    let id = Uuid::now_v7();
    let path = dir.join(format!("{id}.md"));
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| JobError::Permanent(format!("写原料失败: {e}")))?;

    // RETURNING id：sha 冲突时返回旧行 id（payload 必须用它——曾因用新生成
    // uuid 导致 analyze 读不到原料行 no-rows 死循环，Phase 7 审计修复）
    let row = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO wiki_sources (id, sha256, raw_path, title, status) \
         VALUES ($1, $2, $3, $4, 'pending') \
         ON CONFLICT (sha256) DO UPDATE SET title = EXCLUDED.title, status = 'pending' RETURNING id",
    )
    .bind(id)
    .bind(&sha)
    .bind(path.to_string_lossy().as_ref())
    .bind(title)
    .fetch_one(queue.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let real_id = row.0;

    // 冲突行的新原料内容以返回的 id 落盘（read_source 按 id 找路径）
    if real_id != id {
        let _ = tokio::fs::remove_file(&path).await;
        let path = dir.join(format!("{real_id}.md"));
        tokio::fs::write(&path, text)
            .await
            .map_err(|e| JobError::Permanent(format!("写原料失败: {e}")))?;
        let _ = tokio::fs::remove_file(&path.with_extension("md")).await; // no-op clarity
        sqlx::query("UPDATE wiki_sources SET raw_path = $2 WHERE id = $1")
            .bind(real_id)
            .bind(path.to_string_lossy().as_ref())
            .execute(queue.pool())
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }

    queue
        .enqueue(
            JobTemplate::new("wiki_analyze")
                .with_payload(json!({"source_id": real_id}))
                .with_idempotency_key(format!("wiki-analyze-{real_id}")),
        )
        .await?;
    Ok((real_id, false))
}

/// 第一步：分析。source 全文 + 既有 index → 结构化分析（存 wiki_sources.status + 事件）。
pub async fn analyze_job(
    ctx: JobContext,
    llm: crate::service::LlmRef,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let source_id: Uuid = ctx
        .job
        .payload
        .0
        .get("source_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 source_id".into()))?;

    sqlx::query("UPDATE wiki_sources SET status = 'processing' WHERE id = $1")
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let text = read_source(pool, source_id).await?;
    let index = read_index(pool).await?;

    let user = format!("== 现有页面目录 ==\n{index}\n\n== 源文档 ==\n{text}");
    let out = agent_memory_distill::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::WikiAnalysis,
        &prompts::analysis_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    ctx.emit("分析完成", Some(out.clone())).await.ok();

    // 链式入队生成
    ctx.enqueue_next(
        JobTemplate::new("wiki_generate")
            .with_payload(json!({"source_id": source_id, "analysis": out, "source_title": title_of(pool, source_id).await?}))
            .with_idempotency_key(format!("wiki-generate-{source_id}")),
    )
    .await?;
    Ok(json!({"source_id": source_id}))
}

/// 第二步：生成/更新页面 + 索引 + 链接图 + 嵌入。
pub async fn generate_job(
    ctx: JobContext,
    llm: crate::service::LlmRef,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let source_id: Uuid = ctx
        .job
        .payload
        .0
        .get("source_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 source_id".into()))?;
    let analysis = ctx
        .job
        .payload
        .0
        .get("analysis")
        .cloned()
        .ok_or_else(|| JobError::Permanent("payload 缺 analysis".into()))?;
    let source_title = ctx
        .job
        .payload
        .0
        .get("source_title")
        .and_then(|v| v.as_str())
        .unwrap_or("未命名源")
        .to_string();

    let text = read_source(pool, source_id).await?;
    let existing_pages = read_index(pool).await?;

    let user = format!(
        "== 分析结果 ==\n{}\n\n== 源文档 ==\n{}\n\n== 既有页面集合（已存在，勿重建）==\n{}",
        serde_json::to_string_pretty(&analysis).unwrap_or_default(),
        text,
        existing_pages
    );
    let out = agent_memory_distill::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::WikiGeneration,
        &prompts::generation_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    let pages = out
        .get("pages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut proposals = 0usize;
    let mut all_slugs: Vec<String> = Vec::new();

    for p in &pages {
        let slug = p
            .get("slug")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let page_type = p
            .get("page_type")
            .and_then(|v| v.as_str())
            .unwrap_or("concept");
        let title = p
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&slug)
            .trim()
            .to_string();
        let content = p
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !is_valid_slug(&slug) || content.is_empty() {
            continue;
        }
        all_slugs.push(slug.clone());

        let existing: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id, origin FROM wiki_pages WHERE slug = $1")
                .bind(&slug)
                .fetch_optional(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;

        let fm = json!({
            "title": title,
            "page_type": page_type,
            "sources": [source_id.to_string()],
            "origin_if_new": "llm",
        });

        match existing {
            None => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version) \
                     VALUES ($1, $2, $3, $4, $5, $6, 'llm', 1)",
                )
                .bind(id)
                .bind(&slug)
                .bind(&title)
                .bind(page_type)
                .bind(&content)
                .bind(sqlx::types::Json(&fm))
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                created += 1;
            }
            Some((_id, origin)) if origin == "human" => {
                // 人写的页面不覆盖 → 提案（存新内容于 proposal 事件）
                proposals += 1;
                ctx.emit(
                    "人工页面更新提案（待审核）",
                    Some(json!({
                        "page_slug": slug,
                        "proposal_content": content,
                        "current_version_note": "人工编辑页，需 UI 确认后合入",
                    })),
                )
                .await
                .ok();
            }
            Some((id, _)) => {
                // LLM 页面：合并 sources + 版本递增
                sqlx::query(
                    "UPDATE wiki_pages SET \
                        content = $2, \
                        frontmatter = jsonb_set(frontmatter, '{sources}', \
                            (SELECT COALESCE(jsonb_agg(DISTINCT s), '[]'::jsonb) FROM \
                                (SELECT jsonb_array_elements_text(frontmatter->'sources') AS s FROM wiki_pages WHERE id = $1 \
                                 UNION ALL SELECT $3::text) sub)), \
                        version = version + 1, updated_at = now() \
                     WHERE id = $1",
                )
                .bind(id)
                .bind(&content)
                .bind(source_id.to_string())
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                updated += 1;
            }
        }
    }

    // 链接图：本批页面的 wikilinks + 到既有页的边
    rebuild_links(pool, &all_slugs).await?;

    // 索引 + 日志 + overview 维护
    update_index_and_log(pool, source_id, &source_title, created, updated, proposals).await?;

    // 新/变页嵌入
    let texts: Vec<String> = collect_page_texts(pool, &all_slugs).await?;
    if !texts.is_empty()
        && let Ok(emb) = llm.embed(&texts, ctx.job.id).await
    {
        write_embeddings(pool, &all_slugs, emb).await?;
    }

    sqlx::query("UPDATE wiki_sources SET status = 'ready', last_ingested_at = now() WHERE id = $1")
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    ctx.emit(
        &format!("Wiki 生成：新建 {created} / 更新 {updated} / 提案 {proposals}"),
        Some(json!({"created": created, "updated": updated, "proposals": proposals})),
    )
    .await
    .ok();

    Ok(
        json!({"created": created, "updated": updated, "proposals": proposals, "source_id": source_id}),
    )
}

async fn title_of(pool: &sqlx::PgPool, id: Uuid) -> Result<String, JobError> {
    let t: Option<Option<String>> =
        sqlx::query_scalar("SELECT title FROM wiki_sources WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(t.flatten().unwrap_or_else(|| "未命名源".into()))
}

async fn read_source(pool: &sqlx::PgPool, id: Uuid) -> Result<String, JobError> {
    let path: String = sqlx::query_scalar("SELECT raw_path FROM wiki_sources WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| JobError::Permanent(format!("读原料失败: {e}")))
}

async fn read_index(pool: &sqlx::PgPool) -> Result<String, JobError> {
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug) FROM wiki_pages ORDER BY updated_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(pages
        .into_iter()
        .map(|(slug, title)| format!("- [[{slug}]] {title}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 重建给定页面的出边（wikilink 边）。
async fn rebuild_links(pool: &sqlx::PgPool, slugs: &[String]) -> Result<(), JobError> {
    for slug in slugs {
        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM wiki_pages WHERE slug = $1")
                .bind(slug)
                .fetch_optional(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?
                .flatten();
        let Some(content) = content else { continue };
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1")
            .bind(slug)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        for target in extract_wikilinks(&content) {
            sqlx::query(
                "INSERT INTO wiki_links (from_slug, to_slug, weight) VALUES ($1, $2, 3.0) \
                 ON CONFLICT (from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(&target)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        }
    }
    Ok(())
}

/// index/log/overview 系统页维护。
async fn update_index_and_log(
    pool: &sqlx::PgPool,
    source_id: Uuid,
    source_title: &str,
    created: usize,
    updated: usize,
    proposals: usize,
) -> Result<(), JobError> {
    // index：全部页面目录
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug) FROM wiki_pages \
         WHERE page_type NOT IN ('index','log') ORDER BY updated_at DESC LIMIT 300",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let index_md = format!(
        "# 知识库索引\n\n{}\n",
        pages
            .iter()
            .map(|(s, t)| format!("- [[{s}]] — {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // log：append 一行（存 latest 系统页；全量历史靠 job_events）
    let log_line = format!(
        "- {} ingest `{}` → 新建 {created} / 更新 {updated} / 提案 {proposals}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M"),
        source_title
    );
    let prev_log: Option<String> = sqlx::query_scalar(
        "SELECT content FROM wiki_pages WHERE slug = 'log' AND page_type = 'log'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .flatten();
    let log_md = format!(
        "{}\n{}",
        prev_log.unwrap_or_else(|| "# 操作日志\n".into()),
        log_line
    );

    upsert_system_page(pool, "index", "index", "索引", &index_md).await?;
    upsert_system_page(pool, "log", "log", "日志", &log_md).await?;
    let _ = source_id;
    Ok(())
}

async fn upsert_system_page(
    pool: &sqlx::PgPool,
    slug: &str,
    page_type: &str,
    title: &str,
    content: &str,
) -> Result<(), JobError> {
    sqlx::query(
        "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version) \
         VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, 'llm', 1) \
         ON CONFLICT (slug) DO UPDATE SET content = $5, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(slug)
    .bind(title)
    .bind(page_type)
    .bind(content)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

async fn collect_page_texts(
    pool: &sqlx::PgPool,
    slugs: &[String],
) -> Result<Vec<String>, JobError> {
    let mut out = Vec::new();
    for slug in slugs {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT COALESCE(frontmatter->>'title', slug), content FROM wiki_pages WHERE slug = $1",
        )
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        if let Some((title, content)) = row {
            out.push(format!("{title}\n{content}"));
        }
    }
    Ok(out)
}

async fn write_embeddings(
    pool: &sqlx::PgPool,
    slugs: &[String],
    embeddings: Vec<Vec<f32>>,
) -> Result<(), JobError> {
    for (i, slug) in slugs.iter().enumerate() {
        if let Some(v) = embeddings.get(i) {
            sqlx::query(
                "UPDATE wiki_pages SET embedding = $2, tsv = to_tsvector('simple', $3) WHERE slug = $1",
            )
            .bind(slug)
            .bind(pgvector::Vector::from(v.clone()))
            .bind(agent_memory_search::tokenize::tsv_text(slug))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        }
    }
    Ok(())
}

/// 注册 Wiki handler。
pub fn register_handlers(
    runner: agent_memory_jobs::Runner,
    llm: crate::service::LlmRef,
) -> agent_memory_jobs::Runner {
    let l1 = llm.clone();
    let l2 = llm.clone();
    runner
        .register("wiki_analyze", move |ctx| {
            let llm = l1.clone();
            async move { analyze_job(ctx, llm).await }
        })
        .register("wiki_generate", move |ctx| {
            let llm = l2.clone();
            async move { generate_job(ctx, llm).await }
        })
}
