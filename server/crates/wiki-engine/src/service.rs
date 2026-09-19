//! Wiki 服务层：页面 CRUD、图数据、ingest 入口、lint 调用。
//! 多库（0037）：所有公开方法在 `&self` 后带 `lib: Uuid`，SQL 全部按库过滤/写入。

use chrono::{DateTime, Utc};
use engram_jobs::{JobQueue, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::{tsv_query_smart_wiki, tsv_text_wiki};
use sqlx::{FromRow, Row};

/// wiki_pages tsv 覆盖口径（EN-63）：slug + title + content 三合一——slug 复合词
/// 归一后（ai passthrough principle）与标题词都在索引里，搜索任何一段均可命中。
fn page_tsv_text(slug: &str, title: &str, content: &str) -> String {
    tsv_text_wiki(&format!("{slug} {title} {content}"))
}
use uuid::Uuid;

use crate::ingest;
use crate::lint;

/// LLM 引用（ingest job 注入）。
pub type LlmRef = std::sync::Arc<dyn engram_distill::llm_port::DistillLlm>;

#[derive(Debug, thiserror::Error)]
pub enum WikiError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<sqlx::Error> for WikiError {
    fn from(e: sqlx::Error) -> Self {
        WikiError::Storage(e.to_string())
    }
}

impl From<engram_jobs::types::JobError> for WikiError {
    fn from(e: engram_jobs::types::JobError) -> Self {
        WikiError::Storage(e.to_string())
    }
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WikiPageDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub page_type: String,
    /// 目录树层级（/ 分隔多级，Obsidian 式文件夹）
    pub folder: String,
    pub content: String,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub origin: String,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
}

/// 页面版本快照行（列表用——不带正文，防一次拖回全史）。
#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WikiPageVersionDto {
    pub id: Uuid,
    pub slug: String,
    pub version: i32,
    pub title: String,
    pub page_type: String,
    pub folder: String,
    pub origin: String,
    /// 该版正文字符数（决定是否值得回读全文）
    pub content_chars: i64,
    pub created_at: DateTime<Utc>,
}

/// 查询缺口行（批次② 查询日志飞轮）——零命中/低分查询即内容缺口。
#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct QueryGapDto {
    pub query: String,
    pub calls: i32,
    pub zero_calls: i32,
    pub low_calls: i32,
    pub last_top_score: Option<f32>,
    pub last_queried_at: DateTime<Utc>,
}

/// 批次② 查询日志低分阈值：≈单通道首位 RRF 水平（1/61≈0.0164）。
/// top_score 低于它 = 只靠单通道勉强命中，召回质量存疑。
const QUERY_LOG_LOW_SCORE: f64 = 0.017;

/// 审计缺陷④（2026-09-20）：存量页向量回填——embedding IS NULL 的非系统页批量补嵌。
/// 织入尾部（自愈）与 repair job（手动触发）调用；cap 50/次防热路径长尾。返回补嵌页数。
pub async fn backfill_page_embeddings(
    pool: &sqlx::PgPool,
    llm: Option<&LlmRef>,
    lib: Uuid,
) -> Result<usize, WikiError> {
    let Some(llm) = llm else {
        return Ok(0); // 未注入 LLM 通道——无嵌入能力，静默跳过
    };
    let pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages \
         WHERE library_id = $1 AND embedding IS NULL \
           AND page_type NOT IN ('index','log','overview') \
         ORDER BY updated_at DESC LIMIT 50",
    )
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| WikiError::Storage(e.to_string()))?;
    if pages.is_empty() {
        return Ok(0);
    }
    let texts: Vec<String> = pages
        .iter()
        .map(|(_, title, content)| format!("{title}\n{content}"))
        .collect();
    let emb = llm
        .embed(&texts, Uuid::now_v7())
        .await
        .map_err(|e| WikiError::Storage(e.to_string()))?;
    if emb.len() != texts.len()
        || emb
            .iter()
            .any(|v| v.len() != engram_distill::llm_port::embedding_dimensions() as usize)
    {
        return Err(WikiError::Storage(
            "补嵌响应与批次不符——本批向量全部放弃（K4 守卫同款语义）".into(),
        ));
    }
    for (i, (slug, _, _)) in pages.iter().enumerate() {
        sqlx::query("UPDATE wiki_pages SET embedding = $3 WHERE slug = $1 AND library_id = $2")
            .bind(slug)
            .bind(lib)
            .bind(pgvector::Vector::from(emb[i].clone()))
            .execute(pool)
            .await
            .map_err(|e| WikiError::Storage(e.to_string()))?;
    }
    Ok(pages.len())
}

/// 批次② 查询日志：检索后 UPSERT（calls 累计；零命中/低分计数——缺口挖掘原料）。
async fn log_query(
    pool: &sqlx::PgPool,
    lib: Uuid,
    query: &str,
    hits: usize,
    top_score: Option<f64>,
) -> Result<(), sqlx::Error> {
    let top = top_score.unwrap_or(0.0) as f32;
    let zero = (hits == 0) as i32;
    let low = (hits > 0 && top_score.is_some_and(|s| s < QUERY_LOG_LOW_SCORE)) as i32;
    sqlx::query(
        "INSERT INTO wiki_query_log (id, library_id, query, calls, zero_calls, low_calls, last_top_score) \
         VALUES ($1, $2, $3, 1, $4, $5, $6) \
         ON CONFLICT (library_id, query) DO UPDATE SET \
            calls = wiki_query_log.calls + 1, \
            zero_calls = wiki_query_log.zero_calls + $4, \
            low_calls = wiki_query_log.low_calls + $5, \
            last_top_score = $6, last_queried_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(lib)
    .bind(query)
    .bind(zero)
    .bind(low)
    .bind(top)
    .execute(pool)
    .await
    .map(|_| ())
}

/// page_type → 目录树默认文件夹（Obsidian 式目录树层级）。
pub fn folder_for_type(page_type: &str) -> &'static str {
    match page_type {
        "entity" => "实体",
        "concept" => "概念",
        "source" => "来源",
        "synthesis" => "综合",
        "comparison" => "对比",
        "queries" => "查询",
        "overview" => "总览",
        "index" => "索引",
        "log" => "日志",
        "purpose" => "目标",
        _ => "",
    }
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct GraphDto {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Louvain 社区信息（id → 凝聚度）
    #[serde(default)]
    pub communities: Vec<CommunityInfo>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct CommunityInfo {
    pub id: usize,
    /// 社区首位成员 slug（与 insights 口径一致：成员表首个）
    pub top_slug: String,
    pub size: usize,
    pub cohesion: f64,
    /// 稀疏社区（后端判定：cohesion<0.15 且成员≥3，与 insights 同口径）——
    /// 前端图例直接消费，避免本地重复实现漂移
    pub sparse: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct GraphNode {
    pub slug: String,
    pub title: String,
    pub page_type: String,
    /// Louvain 社区 id（着色切换用）
    #[serde(default)]
    pub community: usize,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct GraphEdge {
    pub from_slug: String,
    pub to_slug: String,
    pub weight: f32,
}

#[derive(Clone)]
pub struct WikiService {
    pool: sqlx::PgPool,
    queue: JobQueue,
    /// W2：检索向量通道需要查询嵌入（无 provider 时退纯 FTS）
    registry: ProviderRegistry,
    /// 批次④：LLM rerank 精排通道（None = rerank 参数降级不可用——检索仍正常）
    llm: Option<LlmRef>,
}

/// 每 slug 保留的版本快照上限（与 skill_revisions 同口径）。
pub(crate) const VERSION_KEEP: i64 = 50;

impl WikiService {
    pub fn new(pool: sqlx::PgPool, registry: ProviderRegistry) -> Self {
        Self {
            queue: JobQueue::new(pool.clone()),
            registry,
            pool,
            llm: None,
        }
    }

    /// 批次④：注入 LLM rerank 精排通道（api/mcp 组装点 opt-in）。
    pub fn with_llm(mut self, llm: LlmRef) -> Self {
        self.llm = Some(llm);
        self
    }

    /// 审计缺陷④：存量页向量回填（cap 50/次）——repair job 与织入尾部的自愈入口。
    pub async fn backfill_embeddings(&self, lib: Uuid) -> Result<usize, WikiError> {
        backfill_page_embeddings(&self.pool, self.llm.as_ref(), lib).await
    }

    /// 触发两步 ingest（文本 + 标题）。返回三态（D27）：已就绪跳过 / 在途 / 新入队。
    /// D24：空标题/空文本响亮拒绝（空文本任务曾在队列里滞留不执行、空标题白烧一次 LLM）。
    /// 多库：原料落到指定库（sha 去重也只在库内生效）。
    pub async fn ingest(
        &self,
        lib: Uuid,
        title: &str,
        text: &str,
    ) -> Result<crate::ingest::IngestOutcome, WikiError> {
        if title.trim().is_empty() {
            return Err(WikiError::BadRequest(
                "title 不能为空——织入来源需要可辨认的标题".into(),
            ));
        }
        if text.trim().is_empty() {
            return Err(WikiError::BadRequest(
                "text 不能为空——空文本织入只会浪费 LLM 调用".into(),
            ));
        }
        // 收录判据③（2026-09-19 wiki 收录哲学线，用户拍板）：原料须有实质内容才进织入。
        // 种子日志/单行注记（一行日期+短语）织入只会产出无根页面并污染复核队列。
        const MIN_INGEST_CHARS: usize = 80;
        let trimmed = text.trim();
        if trimmed.chars().count() < MIN_INGEST_CHARS {
            return Err(WikiError::BadRequest(format!(
                "原料正文仅 {} 字，低于织入门槛 {MIN_INGEST_CHARS} 字——种子日志/单行注记不属于知识原料（收录判据③：原料须有实质内容）。如是真实知识请补全正文后再织入",
                trimmed.chars().count()
            )));
        }
        Ok(ingest::enqueue_ingest(&self.queue, lib, title, text).await?)
    }

    /// 从 wiki 文档触发织入（upload 与 URL 通用，2026-09-04 补 URL 兜底）：
    /// raw_path 有 → 重新读取原文件并解析（保留原行为）；
    /// raw_path 空（URL 摄取）→ 用已分块文本按 seq 拼接——此前 URL 文档既不能
    /// --doc-id 手动织入（404）也不会被自动织入静默跳过，两路都收敛到 ingest(title, text)。
    /// 多库：文档按 (id, library_id) 匹配——跨库文档按不存在处理。
    pub async fn ingest_document(
        &self,
        lib: Uuid,
        doc_id: Uuid,
    ) -> Result<crate::ingest::IngestOutcome, WikiError> {
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT title, raw_path, mime FROM wiki_documents WHERE id = $1 AND library_id = $2",
        )
        .bind(doc_id)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?;
        let Some((title, raw_path, mime)) = row else {
            return Err(WikiError::NotFound(format!("文档 {doc_id} 不存在")));
        };
        let text = if raw_path.as_deref().is_some_and(|p| !p.is_empty()) {
            let path = raw_path.unwrap_or_default();
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| WikiError::BadRequest(format!("读文件失败: {e}")))?;
            engram_parsing::parse_bytes(&name, mime.as_deref(), &bytes)
                .map_err(|e| WikiError::BadRequest(e.to_string()))?
        } else {
            // URL 摄取：无本地文件，用 chunks 表已解析文本按序拼接（库内）
            let chunks: Vec<String> = sqlx::query_scalar(
                "SELECT c.content FROM wiki_chunks c \
                 WHERE c.document_id = $1 AND c.library_id = $2 ORDER BY c.seq",
            )
            .bind(doc_id)
            .bind(lib)
            .fetch_all(&self.pool)
            .await?;
            if chunks.is_empty() {
                return Err(WikiError::BadRequest(format!(
                    "文档 {doc_id} 无本地文件且无可织入的分块（可能尚未解析完成）"
                )));
            }
            chunks.join("\n\n")
        };
        self.ingest(lib, &title, &text).await
    }

    /// 页面列表（D28 keyset 分页，单页上限 300）：cursor = 上一页最后一条的
    /// `{updated_at ISO8601}|{id}`，首查不传。ORDER BY 带 id 决稳——
    /// 此前静默截断曾让最老的页面从列表「消失」（graph/lint 却可见）。
    /// 多库：只列指定库的页面（slug 跨库可重名）。
    pub async fn list_pages(
        &self,
        lib: Uuid,
        page_type: Option<&str>,
        limit: i64,
        cursor: Option<&str>,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        let parse_cursor =
            |raw: &str| -> Result<(chrono::DateTime<chrono::Utc>, uuid::Uuid), WikiError> {
                let parts: Vec<&str> = raw.split('|').collect();
                if parts.len() != 2 {
                    return Err(WikiError::BadRequest(format!(
                        "cursor 非法（收到 {raw:?}）——期望 {{updated_at ISO8601}}|{{id}}，取上一页最后一条构造"
                    )));
                }
                let ts = chrono::DateTime::parse_from_rfc3339(parts[0].trim())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .map_err(|_| {
                        WikiError::BadRequest(format!(
                            "cursor 时间无法解析（收到 {:?}）——期望 ISO8601",
                            parts[0]
                        ))
                    })?;
                let id = uuid::Uuid::parse_str(parts[1].trim()).map_err(|_| {
                    WikiError::BadRequest(format!("cursor id 不是合法 UUID（收到 {:?}）", parts[1]))
                })?;
                Ok((ts, id))
            };
        match cursor {
            None | Some("") => Ok(sqlx::query_as::<_, WikiPageDto>(
                "SELECT * FROM wiki_pages \
                     WHERE library_id = $1 AND ($2::text IS NULL OR page_type = $2) \
                       AND page_type NOT IN ('log') \
                     ORDER BY updated_at DESC, id DESC LIMIT $3",
            )
            .bind(lib)
            .bind(page_type)
            .bind(limit.min(300))
            .fetch_all(&self.pool)
            .await?),
            Some(raw) => {
                let (ts, id) = parse_cursor(raw)?;
                Ok(sqlx::query_as::<_, WikiPageDto>(
                    "SELECT * FROM wiki_pages \
                     WHERE library_id = $1 AND ($2::text IS NULL OR page_type = $2) \
                       AND page_type NOT IN ('log') \
                       AND (updated_at, id) < ($4::timestamptz, $5::uuid) \
                     ORDER BY updated_at DESC, id DESC LIMIT $3",
                )
                .bind(lib)
                .bind(page_type)
                .bind(limit.min(300))
                .bind(ts)
                .bind(id)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    /// 读单页（库内）。先精确匹配；未中则按「小写 + 空格转连字符」宽容重查——LLM 生成正文时
    /// 常把双链写成标题原文（[[Rust 异步运行时]]），与真实 slug（rust-异步运行时）
    /// 只差大小写和分隔符，精确匹配 404 后点过去就"没反应"。
    /// R 报告 P1-11 双寻址：slug 未中再按 title 精确兜底（标题寻址）。
    pub async fn get_page(&self, lib: Uuid, slug: &str) -> Result<WikiPageDto, WikiError> {
        sqlx::query_as::<_, WikiPageDto>(
            "SELECT * FROM wiki_pages \
             WHERE (slug = $1 OR slug = lower(replace($1, ' ', '-')) OR title = $1) \
               AND library_id = $2 \
             ORDER BY (slug = $1) DESC, (title = $1) DESC LIMIT 1",
        )
        .bind(slug)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| WikiError::NotFound(format!("页面 {slug} 不存在")))
    }

    /// slug/title 宽容解析成真实 slug（删除/版本操作用，与 get_page 同一匹配口径；库内）。
    async fn resolve_slug(&self, lib: Uuid, slug_or_title: &str) -> Result<String, WikiError> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT slug FROM wiki_pages \
             WHERE (slug = $1 OR slug = lower(replace($1, ' ', '-')) OR title = $1) \
               AND library_id = $2 \
             ORDER BY (slug = $1) DESC LIMIT 1",
        )
        .bind(slug_or_title)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| WikiError::NotFound(format!("页面 {slug_or_title} 不存在")))
    }

    /// 人工编辑：origin=human、版本递增、重嵌入。folder 可选（None=保持原值/默认空）。
    /// via 可选（S-7）：执行者标记（如 "ai"）——落 frontmatter.via，区分真人编辑与 AI 代执行。
    /// 多库：快照/页面/链接全部按 (library_id, slug) 操作。
    pub async fn put_page(
        &self,
        lib: Uuid,
        slug: &str,
        title: &str,
        content: &str,
        folder: Option<&str>,
        via: Option<&str>,
    ) -> Result<WikiPageDto, WikiError> {
        if !crate::markup::is_valid_slug(slug) {
            return Err(WikiError::BadRequest(
                "slug 非法：仅允许字母/数字/-/_/·，≤80 字符，不含空格与路径分隔符".into(),
            ));
        }
        // 新建时的 frontmatter：title/sources + via（若有）
        let mut fm_insert = serde_json::json!({"title": title, "sources": []});
        if let Some(v) = via {
            fm_insert["via"] = serde_json::json!(v);
        }
        // 更新时的 frontmatter 合并块：title 恒同步（graph 节点标题依赖，D8）+
        // via（若有）——|| 合并只覆盖指定键，sources 等其余键保留
        let mut fm_merge_obj = serde_json::json!({ "title": title });
        if let Some(v) = via {
            fm_merge_obj["via"] = serde_json::json!(v);
        }
        let fm_merge = fm_merge_obj.to_string();
        // 版本历史（R 报告建议 #5）：覆盖前先把现状快照进 wiki_page_versions——
        // 此前 version 只是计数器，覆盖即失忆。INSERT..SELECT 天然幂等（无旧页 0 行）。
        sqlx::query(
            "INSERT INTO wiki_page_versions (id, library_id, slug, version, title, page_type, folder, content, origin) \
             SELECT $1, $3, slug, version, title, page_type, folder, content, origin \
             FROM wiki_pages WHERE slug = $2 AND library_id = $3",
        )
        .bind(Uuid::now_v7())
        .bind(slug)
        .bind(lib)
        .execute(&self.pool)
        .await?;
        self.prune(lib, slug).await;
        let row = sqlx::query_as::<_, WikiPageDto>(
            "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, $4, 'concept', COALESCE($5, ''), $6, $7::jsonb, 'human', 1, to_tsvector('simple', $8)) \
             ON CONFLICT (library_id, slug) DO UPDATE SET \
                title = $4, content = $6, origin = 'human', \
                folder = COALESCE($5, wiki_pages.folder), \
                frontmatter = wiki_pages.frontmatter || $9::jsonb, \
                version = wiki_pages.version + 1, updated_at = now(), \
                tsv = to_tsvector('simple', $8) \
             RETURNING *",
        )
        .bind(Uuid::now_v7())
        .bind(lib)
        .bind(slug)
        .bind(title)
        .bind(folder)
        .bind(content)
        .bind(fm_insert.to_string())
        .bind(page_tsv_text(slug, title, content))
        .bind(fm_merge)
        .fetch_one(&self.pool)
        .await?;

        // D4：落页后重算本页 wikilinks——graph/孤页检测与 lint 同源（此前
        // put_page 不写 wiki_links，AI 写页的互链对图与 lint 不可见）
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1 AND library_id = $2")
            .bind(slug)
            .bind(lib)
            .execute(&self.pool)
            .await?;
        let mut cross_targets: Vec<(String, String)> = Vec::new();
        for target in crate::markup::extract_wikilinks(content) {
            // R 多库补全：跨库引用 [[lib/slug]] 走 wiki_cross_links（校验目标存在后建链）
            if let Some((to_lib, to_slug)) = crate::markup::split_cross_lib(&target) {
                cross_targets.push((to_lib, to_slug));
                continue;
            }
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                 VALUES ($3, $1, $2, 3.0) \
                 ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(&target)
            .bind(lib)
            .execute(&self.pool)
            .await
            .ok();
        }
        crate::cross_links::sync_page(&self.pool, lib, slug, &cross_targets).await?;
        Ok(row)
    }

    /// 链接图（节点 = 页面，边 = wikilink；含 Louvain 社区 + 凝聚度；库内）。
    pub async fn graph(&self, lib: Uuid) -> Result<GraphDto, WikiError> {
        // D23：排除系统 log 页（list_pages 不可见，图里也不该出现——否则节点无法溯源）
        let nodes: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type FROM wiki_pages \
             WHERE page_type <> 'log' AND library_id = $1",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        // 边随节点过滤：任一端是 log 页的边一并剔除（防悬空引用进社区发现）；边与两端都限库内
        let edges: Vec<(String, String, f32)> = sqlx::query_as(
            "SELECT l.from_slug, l.to_slug, l.weight FROM wiki_links l \
             WHERE l.library_id = $1 \
               AND EXISTS (SELECT 1 FROM wiki_pages f WHERE f.slug = l.from_slug \
                           AND f.page_type <> 'log' AND f.library_id = $1) \
               AND EXISTS (SELECT 1 FROM wiki_pages t WHERE t.slug = l.to_slug \
                           AND t.page_type <> 'log' AND t.library_id = $1)",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        // 社区发现
        let node_slugs: Vec<String> = nodes.iter().map(|(s, _, _)| s.clone()).collect();
        let e64: Vec<(String, String, f64)> = edges
            .iter()
            .map(|(f, t, w)| (f.clone(), t.clone(), *w as f64))
            .collect();
        let comms = crate::community::louvain_communities(&node_slugs, &e64);
        let cohesion = crate::community::community_cohesion(&node_slugs, &e64, &comms);
        // 成员映射（id → 成员 slugs，保持节点顺序）——top_slug/size 真实统计，
        // 与 insights.rs 的 CommunityInfo 口径一致（曾长期 size:0 且缺 top_slug 违约）
        let mut members: std::collections::HashMap<usize, Vec<&str>> =
            std::collections::HashMap::new();
        for slug in &node_slugs {
            members
                .entry(comms.get(slug.as_str()).copied().unwrap_or(0))
                .or_default()
                .push(slug);
        }
        Ok(GraphDto {
            communities: cohesion
                .into_iter()
                .map(|(id, cohesion)| {
                    let m = members.get(&id);
                    let size = m.map_or(0, |v| v.len());
                    CommunityInfo {
                        id,
                        top_slug: m
                            .and_then(|v| v.first().copied())
                            .unwrap_or_default()
                            .to_string(),
                        size,
                        cohesion,
                        sparse: size >= crate::community::SPARSE_MIN_SIZE
                            && cohesion < crate::community::SPARSE_COHESION,
                    }
                })
                .collect(),
            nodes: nodes
                .into_iter()
                .map(|(slug, title, page_type)| GraphNode {
                    community: comms.get(slug.as_str()).copied().unwrap_or(0),
                    slug,
                    title,
                    page_type,
                })
                .collect(),
            edges: edges
                .into_iter()
                .map(|(from_slug, to_slug, weight)| GraphEdge {
                    from_slug,
                    to_slug,
                    weight,
                })
                .collect(),
        })
    }

    pub async fn lint(&self, lib: Uuid) -> Result<lint::LintReport, WikiError> {
        Ok(lint::lint(&self.pool, lib).await?)
    }

    /// 入队语义 lint（lint_deep）任务——LLM 深度检查矛盾/过时/缺页，产出入人审队列。
    pub async fn lint_deep_enqueue(
        &self,
        lib: Uuid,
        slugs: Option<Vec<String>>,
    ) -> Result<Uuid, WikiError> {
        Ok(crate::lint_deep::enqueue(&self.pool, lib, slugs).await?)
    }

    /// 内容目录（karpathy LLM Wiki 的 index 页等价物）：按 page_type 分组的动态聚合，
    /// 只读不落库。每页含入链数与首段摘要——人读与 LLM 导航双用途。
    pub async fn index(&self, lib: Uuid) -> Result<serde_json::Value, WikiError> {
        let rows: Vec<(String, String, String, String, Option<i64>)> = sqlx::query_as(
            "SELECT p.page_type, p.slug, p.title, \
                    COALESCE(split_part(left(regexp_replace(p.content, E'[\\n\\r]+', ' ', 'g'), 160), '。', 1), '') AS summary, \
                    (SELECT count(*)::bigint FROM wiki_links l WHERE l.to_slug = p.slug AND l.library_id = p.library_id) AS inlinks \
             FROM wiki_pages p WHERE p.library_id = $1 AND p.page_type <> 'log' \
             ORDER BY p.page_type, p.slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let mut groups: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
            std::collections::BTreeMap::new();
        for (page_type, slug, title, summary, inlinks) in rows {
            groups
                .entry(page_type)
                .or_default()
                .push(serde_json::json!({
                    "slug": slug, "title": title,
                    "summary": summary, "inlinks": inlinks.unwrap_or(0),
                }));
        }
        let pages: serde_json::Map<String, serde_json::Value> = groups
            .into_iter()
            .map(|(k, v)| (k, serde_json::json!(v)))
            .collect();
        Ok(serde_json::json!({ "groups": pages }))
    }

    /// 问答/分析产物归档（karpathy LLM Wiki：好答案不该消失在聊天记录里）——
    /// 以 page_type=analysis 落页（0040）（复用 put_page 的版本快照与 wikilinks 重算），
    /// 再对 related 页面补双向链接（归档页 ↔ 相关页）。
    pub async fn archive_answer(
        &self,
        lib: Uuid,
        slug: &str,
        title: &str,
        content: &str,
        related: &[String],
    ) -> Result<WikiPageDto, WikiError> {
        if !crate::markup::is_valid_slug(slug) {
            return Err(WikiError::BadRequest(
                "slug 非法：仅允许字母/数字/-/_/·，≤80 字符，不含空格与路径分隔符".into(),
            ));
        }
        let mut page = self
            .put_page(lib, slug, title, content, None, Some("archive"))
            .await?;
        // 归档页固定为 analysis 类型（put_page 硬编码 concept，这里矫正；analysis 由 0040 加入 CHECK）
        sqlx::query("UPDATE wiki_pages SET page_type = 'analysis' WHERE id = $1")
            .bind(page.id)
            .execute(&self.pool)
            .await?;
        page.page_type = "analysis".into();
        // related 双向链接（归档页 ↔ 相关页；目标不存在时跳过该条——与 wikilink 死链语义一致，由 lint 报告）
        for target in related {
            if target == slug {
                continue;
            }
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                 VALUES ($3, $1, $2, 3.0) ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(target)
            .bind(lib)
            .execute(&self.pool)
            .await
            .ok();
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                 VALUES ($3, $1, $2, 3.0) ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(target)
            .bind(slug)
            .bind(lib)
            .execute(&self.pool)
            .await
            .ok();
        }
        Ok(page)
    }

    /// 提案合入（人审通过：把 job_events 里的 proposal 内容写入页面）。
    pub async fn apply_proposal(
        &self,
        lib: Uuid,
        slug: &str,
        content: &str,
        title: &str,
        via: Option<&str>,
    ) -> Result<WikiPageDto, WikiError> {
        // human 合入：保持 origin=human 语义（人确认的内容）；via 落 frontmatter 区分执行者
        self.put_page(lib, slug, title, content, None, via).await
    }

    /// Wiki 检索（FTS + 向量 RRF 融合；W2：向量通道落地；库内检索）。
    /// purpose 注入：检索走 LLM 时（AI 客户端读 query_context.purpose）提供方向意图——对齐 llm_wiki 的 query 注入。
    pub async fn search(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        self.search_opts(lib, query, limit, false).await
    }

    /// 批次④：带 LLM rerank 精排的检索（费用不敏感拍板；LLM 失败降级原序）。
    pub async fn search_reranked(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        self.search_opts(lib, query, limit, true).await
    }

    async fn search_opts(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
        rerank: bool,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        // K7：单字/纯标点无 token → 短路空结果（不再空跑 to_tsquery）
        if !engram_search::tokenize::has_query_tokens(query) {
            return Ok(vec![]);
        }
        let tsq = tsv_query_smart_wiki(query, 3);
        let limit = limit.min(50);

        // W2：查询向量（无 provider / 嵌入失败 → None → 纯 FTS）；L6：经记账门面
        let qv: Option<Vec<f32>> = self
            .registry
            .embed_for(
                Purpose::Embed,
                vec![query.to_string()],
                Some(engram_distill::llm_port::embedding_dimensions()),
                None,
            )
            .await
            .ok()
            .and_then(|r| r.embeddings.first().cloned());

        // 批次⑤：初召回扩到 2×limit（给图扩展留空间），召回后沿双链 2-hop 带衰减重排
        let fetch_n = (limit * 2).min(100);
        // 批次②：真实 RRF 融合分（双通道 rank 归一和）——查询日志的区分度信号；
        // 召回序位分（1/61）对所有第一名恒同、无区分度（审计缺陷③修正）
        let result = if let Some(qv) = qv {
            // FTS + ANN 双候选 + RRF 融合（与 wiki 文档同款模式）；
            // CTE 与外层都按 library_id 过滤——slug 跨库可重名，外层不过滤会串库
            let raw = sqlx::query(
                "WITH fts AS (SELECT slug, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
                 FROM wiki_pages, to_tsquery('simple', $2) q WHERE tsv @@ q AND library_id = $1 \
                   AND page_type NOT IN ('index','log','overview') LIMIT 100), \
                 vec AS (SELECT slug, ROW_NUMBER() OVER (ORDER BY embedding <=> $3) AS rank \
                 FROM wiki_pages WHERE embedding IS NOT NULL AND library_id = $1 \
                   AND page_type NOT IN ('index','log','overview') LIMIT 100) \
                 SELECT p.*, (COALESCE(1.0/(60 + fts.rank), 0) + COALESCE(1.0/(60 + vec.rank), 0))::float8 AS rrf_score \
                 FROM wiki_pages p \
                 LEFT JOIN fts ON fts.slug = p.slug \
                 LEFT JOIN vec ON vec.slug = p.slug \
                 WHERE p.library_id = $1 AND (fts.slug IS NOT NULL OR vec.slug IS NOT NULL) \
                 ORDER BY (COALESCE(1.0/(60 + fts.rank), 0) + COALESCE(1.0/(60 + vec.rank), 0)) DESC \
                 LIMIT $4",
            )
            .bind(lib)
            .bind(tsq)
            .bind(pgvector::Vector::from(qv))
            .bind(fetch_n)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| WikiError::Storage(e.to_string()))?;
            let mut rows: Vec<WikiPageDto> = Vec::with_capacity(raw.len());
            let mut top_rrf: Option<f64> = None;
            for (i, r) in raw.iter().enumerate() {
                if i == 0 {
                    top_rrf = r.try_get::<f64, _>("rrf_score").ok();
                }
                rows.push(WikiPageDto::from_row(r).map_err(|e| WikiError::Storage(e.to_string()))?);
            }
            let direct_hits = rows.len();
            let (out, _graph_top) = self.rerank_with_graph(lib, rows, limit).await?;
            (out, top_rrf, direct_hits)
        } else {
            let rows = sqlx::query_as::<_, WikiPageDto>(
                "SELECT * FROM wiki_pages, to_tsquery('simple', $2) q \
                 WHERE tsv @@ q AND library_id = $1 \
                   AND page_type NOT IN ('index','log','overview') \
                 ORDER BY ts_rank(tsv, q) DESC LIMIT $3",
            )
            .bind(lib)
            .bind(tsq)
            .bind(fetch_n)
            .fetch_all(&self.pool)
            .await?;
            // FTS-only 降级路径：ts_rank 无跨查询可比量级，不判低分（仅零命中判定有效）
            let direct_hits = rows.len();
            let (out, _graph_top) = self.rerank_with_graph(lib, rows, limit).await?;
            (out, None, direct_hits)
        };
        // 批次② 查询日志飞轮：每次检索 UPSERT（直接命中数=0 才记零命中——图扩展补充层
        // 会让最终结果永不为空，零命中必须看直接召回；低分看真实 RRF 融合分）。
        // best-effort——记录失败不影响检索结果。
        let (mut pages, _top, direct_hits) = result;
        if let Err(e) = log_query(&self.pool, lib, query, direct_hits, _top).await {
            tracing::warn!(error = %e, "检索日志记录失败（不影响检索结果）");
        }
        // 批次④ LLM rerank 精排：top-20 交模型重排（Purpose::SearchRerank；单次不重试——
        // 检索热路径；失败/越界降级原序，对齐后端增强线 R6 语义）
        if rerank
            && pages.len() > 1
            && let Some(llm) = &self.llm
        {
            let top: Vec<WikiPageDto> = pages.iter().take(20).cloned().collect();
            let listing: String = top
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "[{}] {}\n{}",
                        i,
                        p.title,
                        p.content.chars().take(120).collect::<String>()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n---\n");
            let system = "你是检索重排序员。给定查询与候选列表（每项带 [索引]），按与查询的相关性从高到低输出索引。只输出 JSON：{\"order\": [索引数组]}，必须包含全部索引且不重复。";
            let user = format!("查询：{query}\n\n候选：\n{listing}");
            match llm
                .chat_json(
                    engram_llm::types::Purpose::SearchRerank,
                    system,
                    &user,
                    Uuid::now_v7(),
                )
                .await
            {
                Ok(v) => {
                    let idx: Vec<usize> = v
                        .get("order")
                        .and_then(|o| o.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_u64().map(|n| n as usize))
                                .collect()
                        })
                        .unwrap_or_default();
                    let n = top.len();
                    if idx.len() == n && idx.iter().all(|&i| i < n) {
                        let mut reordered: Vec<WikiPageDto> =
                            idx.iter().map(|&i| top[i].clone()).collect();
                        if pages.len() > n {
                            reordered.extend(pages.into_iter().skip(n));
                        }
                        pages = reordered;
                    } else {
                        tracing::warn!("wiki rerank：order 长度/索引越界，降级原序");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "wiki rerank 失败，降级原序"),
            }
        }
        Ok(pages)
    }

    /// 图扩展重排（批次⑤）：初召回（已按 RRF/ts_rank 排序）→ 沿双链 2-hop 带衰减扩展 →
    /// 合并重排截 limit。seed 分用召回顺序近似 RRF（1/(60+rank)，与真实 RRF 分单调一致）；
    /// 源重叠等 4 信号已在 wiki_links.weight（relevance::rebuild_weights 每次 ingest 后重算），
    /// 扩展 bonus 因此天然带源重叠权重。
    async fn rerank_with_graph(
        &self,
        lib: Uuid,
        rows: Vec<WikiPageDto>,
        limit: i64,
    ) -> Result<(Vec<WikiPageDto>, Option<f64>), WikiError> {
        let seeds: Vec<(String, f64)> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.slug.clone(), 1.0 / (60.0 + i as f64 + 1.0)))
            .collect();
        let adjacency = self.load_adjacency(lib).await?;
        let expanded = crate::relevance::graph_expand_scores(&seeds, &adjacency);
        // 直接命中保留 RRF 序；扩展分 tie-break 实测调参史（基准集为尺）：
        // 0.05× → 枢纽页霸榜雪崩（92.9%→7.1%）；0.02× → MRR 0.783 仍低于基线 0.789（微扰超阈值）；
        // **0.0× → 精确持平基线**（审计缺陷①修正：不降级是硬约束）。图扩展的主价值在补充层
        // （初召回漏掉的双链强关联页）与源重叠边权（rebuild_weights），不在改排直接命中；
        // 需要质量上限的场景用 rerank（实测 100%/0.918）。
        const BONUS_SCALE: f64 = 0.0;
        let mut scored: std::collections::HashMap<String, f64> = seeds.into_iter().collect();
        for (slug, bonus) in &expanded {
            if let Some(s) = scored.get_mut(slug) {
                *s += bonus * BONUS_SCALE;
            }
        }
        let mut primary: Vec<(String, f64)> = scored.into_iter().collect();
        primary.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        // 第二梯队：图扩展捞回的补充页（语义检索漏掉但双链强关联），排直接命中之后
        let primary_set: std::collections::HashSet<&String> =
            primary.iter().map(|(s, _)| s).collect();
        let mut secondary: Vec<(String, f64)> = expanded
            .into_iter()
            .filter(|(s, _)| !primary_set.contains(s))
            .collect();
        secondary.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut order: Vec<(String, f64)> = primary;
        order.extend(secondary);
        order.truncate(limit.max(0) as usize);
        if order.is_empty() {
            return Ok((vec![], None));
        }
        let top_score = order.first().map(|(_, s)| *s);
        // 回查 DTO（图扩展捞回的页不在初召回 rows 里；带系统页守卫——扩展不捞 index/log/overview）
        let slugs: Vec<String> = order.iter().map(|(s, _)| s.clone()).collect();
        let dtos: Vec<WikiPageDto> = sqlx::query_as(
            "SELECT * FROM wiki_pages WHERE slug = ANY($1) AND library_id = $2 \
             AND page_type NOT IN ('index','log','overview')",
        )
        .bind(&slugs)
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let pos: std::collections::HashMap<String, usize> = order
            .iter()
            .enumerate()
            .map(|(i, (s, _))| (s.clone(), i))
            .collect();
        let mut out: Vec<WikiPageDto> = dtos
            .into_iter()
            .filter(|d| pos.contains_key(&d.slug))
            .collect();
        out.sort_by_key(|d| pos.get(&d.slug).copied().unwrap_or(usize::MAX));
        Ok((out, top_score))
    }

    /// 库内双链无向邻接表（图扩展用；百页级内存直载）。
    /// weight 列是 FLOAT4——SQL 层 ::float8 转，避免 sqlx 运行时解码类型错配（2026-09-19 实测炸点）。
    async fn load_adjacency(
        &self,
        lib: Uuid,
    ) -> Result<std::collections::HashMap<String, Vec<(String, f64)>>, WikiError> {
        let edges: Vec<(String, String, f64)> = sqlx::query_as(
            "SELECT from_slug, to_slug, weight::float8 FROM wiki_links WHERE library_id = $1",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let mut adj: std::collections::HashMap<String, Vec<(String, f64)>> =
            std::collections::HashMap::new();
        for (f, t, w) in edges {
            adj.entry(f.clone()).or_default().push((t.clone(), w));
            adj.entry(t).or_default().push((f, w));
        }
        Ok(adj)
    }

    /// 存量页 tsv 重刷（EN-63）：内容页、slug+title+content、wiki 分词变体。
    ///
    /// 排除 index/log/overview 系统页——它们是目录/日志结构页不是内容（insights/lint/
    /// cascade 等全部读者都排除它们），且 overview 页聚合了几乎全库正文、是关键词汤；
    /// 历史上它们 tsv 为 NULL 不参与 FTS，重刷若包含会让系统页霸榜（audit 实证回归）。
    /// 幂等（值不变不写）；jieba 分词必须经 Rust，故逐页计算。
    pub async fn backfill_tsv(&self, lib: Uuid) -> Result<u64, WikiError> {
        // 历史残留清理：audit 前的重刷（无排除版）或任何途径给系统页写过的 tsv 必须清 NULL，
        // 否则它们继续参与 FTS 霸榜——仅「不更新」不够
        sqlx::query(
            "UPDATE wiki_pages SET tsv = NULL \
             WHERE library_id = $1 AND page_type IN ('index','log','overview') AND tsv IS NOT NULL",
        )
        .bind(lib)
        .execute(&self.pool)
        .await?;
        let pages: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages \
             WHERE library_id = $1 AND page_type NOT IN ('index','log','overview')",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let mut n = 0u64;
        for (slug, title, content) in &pages {
            let text = page_tsv_text(slug, title, content);
            let r = sqlx::query(
                "UPDATE wiki_pages SET tsv = to_tsvector('simple', $3) \
                 WHERE slug = $1 AND library_id = $2 \
                   AND tsv IS DISTINCT FROM to_tsvector('simple', $3)",
            )
            .bind(slug)
            .bind(lib)
            .bind(&text)
            .execute(&self.pool)
            .await?;
            n += r.rows_affected();
        }
        Ok(n)
    }

    /// 批次② 缺口清单：零命中/低分查询（织入方向与 Deep Research 的输入）。
    pub async fn query_gaps(&self, lib: Uuid, limit: i64) -> Result<Vec<QueryGapDto>, WikiError> {
        Ok(sqlx::query_as::<_, QueryGapDto>(
            "SELECT query, calls, zero_calls, low_calls, last_top_score, last_queried_at \
             FROM wiki_query_log WHERE library_id = $1 AND (zero_calls > 0 OR low_calls > 0) \
             ORDER BY GREATEST(zero_calls, low_calls) DESC, last_queried_at DESC LIMIT $2",
        )
        .bind(lib)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 检索上下文包（query 时 purpose 注入的载体）：purpose + 命中页面，
    /// AI 客户端把 purpose 作为 system context 前缀使用（purpose 取该库的）。
    /// 批次④：rerank 参数化——HTTP/MCP 请求级开关（默认 false，检索框速度优先）。
    pub async fn search_with_purpose(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
        rerank: bool,
    ) -> Result<serde_json::Value, WikiError> {
        let pages = if rerank {
            self.search_reranked(lib, query, limit).await?
        } else {
            self.search(lib, query, limit).await?
        };
        // W-10（2026-09-04）：未设 purpose 时返回 null，与 GET /wiki/purpose 一致；
        // 不再用 purpose_context 的默认模板——「读当前设置」与「注入 LLM」语义分开。
        let purpose = crate::purpose::get_purpose(&self.pool, lib)
            .await
            .ok()
            .flatten();
        Ok(serde_json::json!({
            "purpose": purpose,
            "pages": pages,
        }))
    }

    /// 死任务重跑（UI 辅助）。
    pub async fn reingest(&self, source_id: Uuid) -> Result<(), WikiError> {
        self.queue
            .enqueue(
                JobTemplate::new("wiki_analyze")
                    .with_payload(serde_json::json!({"source_id": source_id})),
            )
            .await?;
        Ok(())
    }

    // ---------- purpose（wiki 灵魂；每库一份，键 wiki_purpose:{lib}） ----------

    pub async fn get_purpose(
        &self,
        lib: Uuid,
    ) -> Result<Option<crate::purpose::Purpose>, WikiError> {
        crate::purpose::get_purpose(&self.pool, lib)
            .await
            .map_err(WikiError::from)
    }

    pub async fn set_purpose(
        &self,
        lib: Uuid,
        p: &crate::purpose::Purpose,
    ) -> Result<(), WikiError> {
        crate::purpose::set_purpose(&self.pool, lib, p)
            .await
            .map_err(WikiError::from)
    }

    // ---------- Review ----------

    pub async fn reviews(
        &self,
        lib: Uuid,
        status: Option<&str>,
    ) -> Result<Vec<crate::review::ReviewItem>, WikiError> {
        let items = crate::review::list_by_status(&self.pool, lib, status)
            .await
            .map_err(WikiError::from)?;
        // 腐烂标注：提案指向的页面已删除 → stale 字段列出已删 slug
        crate::review::annotate_stale(&self.pool, lib, items)
            .await
            .map_err(WikiError::from)
    }

    pub async fn review_resolve(
        &self,
        id: Uuid,
        action: Option<&str>,
        dismiss: bool,
    ) -> Result<(), WikiError> {
        let hit = crate::review::resolve(&self.pool, id, action, dismiss)
            .await
            .map_err(WikiError::from)?;
        if !hit {
            // 未命中（不存在或已处理）按 404 语义返回，并给下一步指引（错误文案三问）
            return Err(WikiError::NotFound(format!(
                "review {id} 不存在或已处理——可 GET /wiki/reviews 查看当前待审列表"
            )));
        }
        Ok(())
    }

    // ---------- queries 页型闭环 ----------

    /// 检索结果/问答 → 直接落 queries 页型（人工归档）→ 同时入队再摄取吸收实体概念（库内）。
    pub async fn archive_query(
        &self,
        lib: Uuid,
        title: &str,
        question: &str,
        answer: &str,
    ) -> Result<bool, WikiError> {
        // W-13（2026-09-04）：同 title 已存档 → 幂等跳过（契约「重复→skipped」），
        // 不再落页 version+1 + 再摄取烧 LLM。
        let slug = format!("query-{title}");
        // D5：slug 或 title 任一命中即幂等跳过（此前仅 slug 检查，title 尾随差异漏网 → 覆盖旧答案）
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM wiki_pages WHERE (slug = $1 OR title = $2) AND library_id = $3",
        )
        .bind(&slug)
        .bind(title)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?;
        if exists.is_some() {
            return Ok(true);
        }
        let ts = chrono::Utc::now().format("%Y-%m-%d");
        let content = format!(
            "# {title}\n\n**问**：{question}\n\n**答**：{answer}\n\n（来源：检索存档 {ts}）"
        );

        // 1) 直接落 queries 页（page_type=queries，origin=human——人触发的存档）
        let fm = serde_json::json!({"title": title, "page_type": "queries", "sources": []});
        sqlx::query(
            "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, $4, 'queries', '查询', $5, $6, 'human', 1, to_tsvector('simple', $7)) \
             ON CONFLICT (library_id, slug) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(lib)
        .bind(&slug)
        .bind(title)
        .bind(&content)
        .bind(sqlx::types::Json(&fm))
        .bind(page_tsv_text(&slug, title, &content))
        .execute(&self.pool)
        .await?;

        // 2) 再摄取（实体概念网络吸收本次问答内容）——D27 三态：仅已就绪算 skipped
        let outcome = crate::ingest::enqueue_ingest(&self.queue, lib, title, &content).await?;
        Ok(outcome.skipped())
    }

    /// write_page 织入钩子（2026-09-13）：AI 写页后自动把页面当原料入队再摄取
    /// （吸收概念/实体/互链，不级联重建全库）——「写入即处理」。
    /// 同内容（sha）去重内建：已 ready 跳过、在途 InFlight、failed 才重提。
    pub async fn auto_ingest_page(
        &self,
        lib: Uuid,
        title: &str,
        content: &str,
    ) -> Result<serde_json::Value, WikiError> {
        let outcome = crate::ingest::enqueue_ingest(&self.queue, lib, title, content).await?;
        let v = match outcome {
            crate::ingest::IngestOutcome::Enqueued(id, job) => serde_json::json!({
                "state": "enqueued", "source_id": id, "job_id": job,
                "hint": "已入队织入（analyze→generate，任务页可见）——概念吸收与互链稍后出现",
            }),
            crate::ingest::IngestOutcome::AlreadyReady(id) => serde_json::json!({
                "state": "already_ingested", "source_id": id,
                "hint": "同内容已织入过（sha 命中）——跳过",
            }),
            crate::ingest::IngestOutcome::InFlight(id, job) => serde_json::json!({
                "state": "in_flight", "source_id": id, "job_id": job,
                "hint": "织入在途（同内容正在处理）",
            }),
        };
        Ok(v)
    }

    /// 存量回填（D4 遗留）：重析全部页面正文重建 wiki_links（库内）。
    /// 修复前写入的页面链接索引缺失——一次性全量重析（幂等，先清后建）。
    pub async fn rebuild_all_links(&self, lib: Uuid) -> Result<u64, WikiError> {
        let pages: Vec<(String, String)> = sqlx::query_as(
            "SELECT slug, content FROM wiki_pages WHERE library_id = $1 ORDER BY slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        sqlx::query("DELETE FROM wiki_links WHERE library_id = $1")
            .bind(lib)
            .execute(&self.pool)
            .await?;
        let mut n = 0;
        for (slug, content) in &pages {
            for target in crate::markup::extract_wikilinks(content) {
                if crate::markup::split_cross_lib(&target).is_some() {
                    continue; // 跨库引用不进库内 wiki_links（graph/孤页检测是库内概念）
                }
                sqlx::query(
                    "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                     VALUES ($3, $1, $2, 3.0) \
                     ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
                )
                .bind(slug)
                .bind(&target)
                .bind(lib)
                .execute(&self.pool)
                .await?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// 删除页面（D10：MCP wiki_delete_page / HTTP DELETE /wiki/pages/{slug}）——
    /// 连带清理双向 wikilinks（图与孤页检测不留幽灵边）。
    /// 删除前快照最后状态进 wiki_page_versions——误删可经 restore_version 重建
    /// （R 报告「删除抹掉全部历史」的回收通道；版本历史本身保留）。库内操作。
    pub async fn delete_page(&self, lib: Uuid, slug: &str) -> Result<bool, WikiError> {
        let slug = self.resolve_slug(lib, slug).await?;
        sqlx::query(
            "INSERT INTO wiki_page_versions (id, library_id, slug, version, title, page_type, folder, content, origin) \
             SELECT $1, $3, slug, version, title, page_type, folder, content, origin \
             FROM wiki_pages WHERE slug = $2 AND library_id = $3",
        )
        .bind(Uuid::now_v7())
        .bind(&slug)
        .bind(lib)
        .execute(&self.pool)
        .await?;
        self.prune(lib, &slug).await;
        let n = sqlx::query("DELETE FROM wiki_pages WHERE slug = $1 AND library_id = $2")
            .bind(&slug)
            .bind(lib)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(WikiError::NotFound(format!("页面 {slug} 不存在")));
        }
        sqlx::query(
            "DELETE FROM wiki_links WHERE (from_slug = $1 OR to_slug = $1) AND library_id = $2",
        )
        .bind(&slug)
        .bind(lib)
        .execute(&self.pool)
        .await?;
        // R 多库补全：跨库引用级联清理（from 侧与 to 侧）
        crate::cross_links::delete_page_cleanup(&self.pool, lib, &slug).await?;
        // 腐烂治理（工单「人审队列腐烂」）：指向该页的 open 提案自动 dismissed（可审计不删数据）
        let _ = crate::review::cascade_dismiss(&self.pool, lib, Some(&slug), None).await;
        Ok(true)
    }

    /// Repair：lint 修而不只报（wiki 收录哲学线工单③）。
    /// 边界三级（roadmap v6）：自动做（变体死链改写 / 去链接化 / ≥3 页引用建 stub / 孤页沿出链回挂）、
    /// 留痕做（同标题重复合并——冗余丢弃或内容并入；delete_page 快照兜底 + 全库链接改指）、
    /// 不做（物理删除有内容的独立页——问用户；语义级重复发现留给 lint_deep + AI 处置）。
    /// 全程确定性（不调 LLM）；页面修改一律走 put_page 语义（版本快照 + frontmatter.via="ai"）。
    pub async fn repair(&self, lib: Uuid) -> Result<crate::repair::RepairReport, WikiError> {
        use crate::repair::{RepairAction, RepairReport, rewrite_links};
        use std::collections::{HashMap, HashSet};
        let mut actions: Vec<RepairAction> = Vec::new();

        // 全库页快照（非 log；slug/title/content 三张 map 是本函数的工作状态）
        let pages: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type, content FROM wiki_pages \
             WHERE page_type <> 'log' AND library_id = $1 ORDER BY slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let checked = pages.len();
        let mut contents: HashMap<String, String> = pages
            .iter()
            .map(|(s, .., c)| (s.clone(), c.clone()))
            .collect();

        // ── 1. 同标题重复合并（留痕做，复用 merge_pages 原语）──
        // primary = 入链最多 → 正文最长（信息最全者为主）。
        let inlinks: Vec<(String, i64)> = sqlx::query_as(
            "SELECT to_slug, count(*) FROM wiki_links WHERE library_id = $1 GROUP BY to_slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let inlink_map: HashMap<String, i64> = inlinks.into_iter().collect();
        let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
        for (s, t, ..) in &pages {
            by_title.entry(t.clone()).or_default().push(s.clone());
        }
        for (title, mut group) in by_title {
            if group.len() < 2 {
                continue;
            }
            group.sort_by_key(|s| {
                std::cmp::Reverse((
                    inlink_map.get(s).copied().unwrap_or(0),
                    contents.get(s).map(|c| c.chars().count()).unwrap_or(0),
                ))
            });
            let primary = group[0].clone();
            for dup in group.drain(1..) {
                let detail = self.merge_pages(lib, &primary, &dup).await?;
                actions.push(RepairAction {
                    action: "merge_duplicate".into(),
                    slug: primary.clone(),
                    detail: format!("同标题「{title}」重复合并：{detail}"),
                });
            }
        }
        // merge_pages 直接落库——从库重载工作状态再进死链段
        let pages: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type, content FROM wiki_pages \
             WHERE page_type <> 'log' AND library_id = $1 ORDER BY slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        contents = pages
            .iter()
            .map(|(s, .., c)| (s.clone(), c.clone()))
            .collect();
        let mut titles: HashMap<String, String> = pages
            .iter()
            .map(|(s, t, ..)| (s.clone(), t.clone()))
            .collect();
        let mut slugs: HashSet<String> = contents.keys().cloned().collect();

        // ── 2. 死链处理（自动做）──
        let mut refs: HashMap<String, Vec<String>> = HashMap::new();
        for (s, c) in &contents {
            for link in crate::markup::extract_wikilinks(c) {
                let (t, _) = crate::repair::split_link(&link);
                if t != *s {
                    refs.entry(t).or_default().push(s.clone());
                }
            }
        }
        let mut squash_map: HashMap<String, String> = HashMap::new();
        for s in &slugs {
            squash_map
                .entry(crate::repair::squash(s))
                .or_insert_with(|| s.clone());
        }
        let mut dead: Vec<String> = refs
            .keys()
            .filter(|t| !slugs.contains(*t) && !t.contains('/'))
            .cloned()
            .collect();
        dead.sort();
        for target in dead {
            let ref_pages: Vec<String> = refs[&target]
                .iter()
                .filter(|p| contents.contains_key(*p))
                .cloned()
                .collect();
            let n_ref = ref_pages.len();
            // a) slug 变体唯一命中 → 全部改写为真实 slug
            let sq = crate::repair::squash(&target);
            let variant_matches: Vec<String> = slugs
                .iter()
                .filter(|s| crate::repair::squash(s) == sq)
                .cloned()
                .collect();
            if let [real] = &variant_matches[..] {
                let real = real.clone();
                let mut n_total = 0usize;
                for p in &ref_pages {
                    if let Some(c) = contents.get_mut(p) {
                        let (nc, n) = rewrite_links(c, &target, Some(&real));
                        if n > 0 {
                            *c = nc;
                            n_total += n;
                            if let Some(t) = titles.get(p) {
                                self.put_page(lib, p, t, c, None, Some("ai")).await?;
                            }
                        }
                    }
                }
                actions.push(RepairAction {
                    action: "rewrite_variant_link".into(),
                    slug: real.clone(),
                    detail: format!(
                        "[[{target}]] 为 slug 变体，{n_ref} 页共 {n_total} 处改写为 [[{real}]]"
                    ),
                });
                continue;
            }
            // b) ≥3 页引用 → 建 stub（「下架不烧书」的补全起点；slug 不合法则退化为去链）
            if n_ref >= 3 {
                let stub_slug = target.to_lowercase().replace(' ', "-");
                if crate::markup::is_valid_slug(&stub_slug) {
                    let content = format!(
                        "# {target}\n\n（stub：repair 自动创建——{n_ref} 个页面引用指向本页但原文缺失，待补全。）"
                    );
                    self.put_page(lib, &stub_slug, &target, &content, None, Some("ai"))
                        .await?;
                    slugs.insert(stub_slug.clone());
                    contents.insert(stub_slug.clone(), content);
                    titles.insert(stub_slug.clone(), target.clone());
                    actions.push(RepairAction {
                        action: "create_stub".into(),
                        slug: stub_slug,
                        detail: format!(
                            "{n_ref} 个页面引用「{target}」但页面缺失——已建 stub 待补全"
                        ),
                    });
                    continue;
                }
            }
            // c) 去链接化（保留文本，摘掉链）
            let mut n_total = 0usize;
            for p in &ref_pages {
                if let Some(c) = contents.get_mut(p) {
                    let (nc, n) = rewrite_links(c, &target, None);
                    if n > 0 {
                        *c = nc;
                        n_total += n;
                        if let Some(t) = titles.get(p) {
                            self.put_page(lib, p, t, c, None, Some("ai")).await?;
                        }
                    }
                }
            }
            actions.push(RepairAction {
                action: "delink".into(),
                slug: target.clone(),
                detail: format!(
                    "[[{target}]] 无匹配页面且仅 {n_ref} 页引用——已去链接化（{n_total} 处）"
                ),
            });
        }

        // ── 3. 孤页沿出链回挂（自动做）──
        // 先统一重建 wiki_links（合并/改写后的真实出链），再找 0 入链页，
        // 把孤页回挂到它第一个「目标存在的库内出链」页的相关区（纯增益：不改不删只加一行）。
        self.rebuild_all_links(lib).await?;
        let system = ["index", "log", "overview"];
        let orphans: Vec<(String, String)> = sqlx::query_as(
            "SELECT p.slug, p.content FROM wiki_pages p \
             WHERE p.page_type <> 'log' AND p.library_id = $1 \
             AND NOT EXISTS (SELECT 1 FROM wiki_links l WHERE l.library_id = $1 AND l.to_slug = p.slug)",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        for (oslug, ocontent) in orphans {
            if system.contains(&oslug.as_str()) || !slugs.contains(&oslug) {
                continue;
            }
            let mut target: Option<String> = None;
            for link in crate::markup::extract_wikilinks(&ocontent) {
                let (t, _) = crate::repair::split_link(&link);
                if t == oslug || !slugs.contains(&t) {
                    continue;
                }
                let exists: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM wiki_pages WHERE slug = $1 AND library_id = $2",
                )
                .bind(&t)
                .bind(lib)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);
                if exists > 0 {
                    target = Some(t);
                    break;
                }
            }
            let Some(target) = target else {
                continue;
            };
            let Some(tc) = contents.get_mut(&target) else {
                continue;
            };
            if tc.contains(&format!("[[{oslug}]]")) {
                continue; // 已有链接，不重复挂
            }
            tc.push_str(&format!("\n\n相关：[[{oslug}]]"));
            if let Some(t) = titles.get(&target) {
                self.put_page(lib, &target, t, tc, None, Some("ai")).await?;
            }
            actions.push(RepairAction {
                action: "attach_orphan".into(),
                slug: oslug.clone(),
                detail: format!("孤页无入链——已回挂到其出链目标「{target}」的相关区"),
            });
        }

        self.rebuild_all_links(lib).await?;
        Ok(RepairReport {
            actions,
            checked_pages: checked,
        })
    }

    /// Merge：新陈代谢的合并原语（wiki 收录哲学线工单④，AI 处置重复 flag 与 repair 共用）。
    /// duplicate 并入 primary：正文为空或为 primary 子串 → 冗余丢弃；否则整段并入「合并自」章节；
    /// primary 自身引用 dup 的链接去链接化（改指会变自链）；全库其他页指向 dup 的链接改指 primary；
    /// delete_page(dup)（版本快照兜底——下架不烧书）。返回人话明细。
    pub async fn merge_pages(
        &self,
        lib: Uuid,
        primary_slug: &str,
        duplicate_slug: &str,
    ) -> Result<String, WikiError> {
        if primary_slug == duplicate_slug {
            return Err(WikiError::BadRequest(
                "primary 与 duplicate 不能是同一页".into(),
            ));
        }
        let primary = self.resolve_slug(lib, primary_slug).await?;
        let dup = self.resolve_slug(lib, duplicate_slug).await?;
        let dup_page = self.get_page(lib, &dup).await?;
        let pri_page = self.get_page(lib, &primary).await?;

        // 1) 内容并入（冗余丢弃 / append 章节）；primary 自身引用 dup → 去链接化
        let dup_c = dup_page.content.trim();
        let discarded = dup_c.is_empty() || pri_page.content.contains(dup_c);
        let mut new_primary = if discarded {
            pri_page.content.clone()
        } else {
            format!(
                "{}\n\n## 合并自〈{}〉（{dup}）\n\n{}",
                pri_page.content, dup_page.title, dup_c
            )
        };
        let (_, n_self) = crate::repair::rewrite_links(&new_primary, &dup, None);
        if n_self > 0 {
            let (nc, _) = crate::repair::rewrite_links(&new_primary, &dup, None);
            new_primary = nc;
        }

        // 2) 全库其他页指向 dup 的链接改指 primary（防合并后新增死链）
        let mut rewrite_total = 0usize;
        let others: Vec<(String, String)> = sqlx::query_as(
            "SELECT slug, content FROM wiki_pages WHERE library_id = $1 AND slug <> $2 AND slug <> $3",
        )
        .bind(lib)
        .bind(&dup)
        .bind(&primary)
        .fetch_all(&self.pool)
        .await?;
        for (slug, content) in &others {
            let (nc, n) = crate::repair::rewrite_links(content, &dup, Some(&primary));
            if n > 0 {
                rewrite_total += n;
                let title: String = sqlx::query_scalar(
                    "SELECT COALESCE(frontmatter->>'title', slug) FROM wiki_pages WHERE slug = $1 AND library_id = $2",
                )
                .bind(slug)
                .bind(lib)
                .fetch_one(&self.pool)
                .await
                .unwrap_or_else(|_| slug.clone());
                self.put_page(lib, slug, &title, &nc, None, Some("ai"))
                    .await?;
            }
        }

        // 3) primary 落合并内容（内容有变才写）
        if new_primary != pri_page.content {
            self.put_page(
                lib,
                &primary,
                &pri_page.title,
                &new_primary,
                None,
                Some("ai"),
            )
            .await?;
        }
        // 4) 删 dup（快照兜底 + 双向链清理 + 提案级联 dismiss）
        self.delete_page(lib, &dup).await?;
        self.rebuild_all_links(lib).await?;
        Ok(format!(
            "{dup} → {primary}（{}，链接改写 {rewrite_total} 处）",
            if discarded {
                "冗余丢弃"
            } else {
                "内容并入"
            }
        ))
    }

    // ---------- 版本历史（R 报告建议 #5：列表 + 回滚；快照按 (library_id, slug) 隔离） ----------

    /// 裁剪旧快照（每 slug 只留最近 VERSION_KEEP 条；best-effort，不影响主流程）。
    async fn prune(&self, lib: Uuid, slug: &str) {
        prune_page_versions(&self.pool, lib, slug, VERSION_KEEP).await;
    }

    /// 页面版本列表（新→旧；不带正文，content_chars 供决策）。
    /// 已删除的页面按 slug 直查快照表——恢复通道不因页面不在而 404。
    pub async fn page_versions(
        &self,
        lib: Uuid,
        slug: &str,
    ) -> Result<Vec<WikiPageVersionDto>, WikiError> {
        let slug = match self.resolve_slug(lib, slug).await {
            Ok(s) => s,
            Err(WikiError::NotFound(_)) => slug.to_string(),
            Err(e) => return Err(e),
        };
        Ok(sqlx::query_as::<_, WikiPageVersionDto>(
            "SELECT id, slug, version, title, page_type, folder, origin, \
             length(content)::bigint AS content_chars, created_at \
             FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 \
             ORDER BY version DESC, created_at DESC",
        )
        .bind(&slug)
        .bind(lib)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 读取一个版本快照的正文（回滚前预览用）。已删除页面按 slug 直查。
    pub async fn page_version_content(
        &self,
        lib: Uuid,
        slug: &str,
        version: i32,
    ) -> Result<String, WikiError> {
        let slug = match self.resolve_slug(lib, slug).await {
            Ok(s) => s,
            Err(WikiError::NotFound(_)) => slug.to_string(),
            Err(e) => return Err(e),
        };
        let row: Option<String> = sqlx::query_scalar(
            "SELECT content FROM wiki_page_versions \
             WHERE slug = $1 AND library_id = $2 AND version = $3",
        )
        .bind(&slug)
        .bind(lib)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| {
            WikiError::NotFound(format!(
                "页面 {slug} 没有版本 {version}——先查 versions 列表取可用版本号"
            ))
        })
    }

    /// 回滚到某个版本快照：以「当前版本 +1」落地（历史不可变，回滚也是新版本）。
    /// 页面已被删除时从快照重建（沿用页型/目录，版本号接续快照史）。库内操作。
    pub async fn restore_page_version(
        &self,
        lib: Uuid,
        slug: &str,
        version: i32,
    ) -> Result<WikiPageDto, WikiError> {
        let snap: (String, String, String, String, i32) = sqlx::query_as(
            "SELECT title, content, page_type, folder, version \
             FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 AND version = $3",
        )
        .bind(slug)
        .bind(lib)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            WikiError::NotFound(format!(
                "没有 {slug}#{version} 的快照——先查 versions 列表取可用版本号"
            ))
        })?;
        let (title, content, page_type, folder, _) = snap;
        match self.get_page(lib, slug).await {
            Ok(_) => {
                // 活页：走 put_page（快照现状 → 落目标内容 → 版本 +1、重算链接）
                self.put_page(lib, slug, &title, &content, Some(&folder), Some("restore"))
                    .await
            }
            Err(WikiError::NotFound(_)) => {
                // 死页重建：版本号接续快照史（避免清零后与历史快照版本撞号）
                let next: i32 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(version), 0) + 1 \
                     FROM (SELECT version FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 \
                           UNION ALL SELECT version FROM wiki_pages WHERE slug = $1 AND library_id = $2) t",
                )
                .bind(slug)
                .bind(lib)
                .fetch_one(&self.pool)
                .await?;
                let fm = serde_json::json!({"title": title, "sources": [], "via": "restore"});
                let row = sqlx::query_as::<_, WikiPageDto>(
                    "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, 'human', $9, to_tsvector('simple', $10)) \
                     RETURNING *",
                )
                .bind(Uuid::now_v7())
                .bind(lib)
                .bind(slug)
                .bind(&title)
                .bind(&page_type)
                .bind(&folder)
                .bind(&content)
                .bind(fm.to_string())
                .bind(next)
                .bind(page_tsv_text(slug, &title, &content))
                .fetch_one(&self.pool)
                .await?;
                for target in crate::markup::extract_wikilinks(&content) {
                    if crate::markup::split_cross_lib(&target).is_some() {
                        continue; // 跨库引用不进库内 wiki_links
                    }
                    sqlx::query(
                        "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                         VALUES ($3, $1, $2, 3.0) \
                         ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
                    )
                    .bind(slug)
                    .bind(&target)
                    .bind(lib)
                    .execute(&self.pool)
                    .await
                    .ok();
                }
                Ok(row)
            }
            Err(e) => Err(e),
        }
    }

    // ---------- 级联删除 ----------

    pub async fn delete_source_cascade(
        &self,
        lib: Uuid,
        source_id: Uuid,
    ) -> Result<crate::cascade::CascadeReport, WikiError> {
        // 源存在性前置检查（NotFound 语义）：按 (library_id, id) 匹配——
        // 源不存在或属于其他库都按 404 处理（多库隔离）
        let hit: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM wiki_sources WHERE id = $1 AND library_id = $2")
                .bind(source_id)
                .bind(lib)
                .fetch_optional(&self.pool)
                .await?;
        if hit.is_none() {
            return Err(WikiError::NotFound(format!("源 {source_id} 不存在")));
        }
        // W-16（2026-09-04）：先取消该源在途的织入任务——否则级联删完，
        // 队列里 analyze/generate 继续跑，边删边产页（测试实测页面 81→84）。
        // running 中的任务若已过写库点仍可能落页，残留由 stale_source lint 报出。
        sqlx::query(
            "UPDATE jobs SET status = 'cancelled', error = '源已删除——织入任务随级联取消', \
             locked_by = NULL, locked_at = NULL \
             WHERE kind IN ('wiki_analyze','wiki_generate') \
             AND status IN ('pending','running') \
             AND payload->>'source_id' = $1::text",
        )
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        let report = crate::cascade::cascade_delete_source(&self.pool, source_id)
            .await
            .map_err(WikiError::from)?;
        // 腐烂治理（工单「人审队列腐烂」）：指向该源的 open 提案自动 dismissed（可审计不删数据）
        let _ = crate::review::cascade_dismiss(&self.pool, lib, None, Some(source_id)).await;
        // 破坏性操作落审计行（与 memory 域「job 行即审计链」同哲学）——best-effort，不阻断返回
        self.audit(
            "wiki_source_cascade_delete",
            serde_json::json!({ "source_id": source_id, "report": report }),
        )
        .await;
        Ok(report)
    }

    // ---------- 图洞察 ----------

    pub async fn insights(&self, lib: Uuid) -> Result<crate::insights::InsightsReport, WikiError> {
        crate::insights::compute_insights(&self.pool, lib)
            .await
            .map_err(WikiError::from)
    }

    pub async fn insight_dismiss(&self, lib: Uuid, key: &str) -> Result<(), WikiError> {
        crate::insights::dismiss(&self.pool, lib, key)
            .await
            .map_err(WikiError::from)
    }

    pub async fn insight_reset(&self, lib: Uuid) -> Result<(), WikiError> {
        crate::insights::reset_dismissals(&self.pool, lib)
            .await
            .map_err(WikiError::from)
    }

    /// 供 API 列出可删的 sources（库内）。
    pub async fn list_sources(
        &self,
        lib: Uuid,
    ) -> Result<Vec<(Uuid, Option<String>, String, String)>, WikiError> {
        Ok(sqlx::query_as(
            "SELECT id, title, status, sha256 FROM wiki_sources \
             WHERE library_id = $1 ORDER BY created_at DESC",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 审计行（清空不吞审计凭证）：破坏性操作落 jobs 成功行，best-effort。
    /// jobs 表不挂库——审计链全库共享，故本方法不引入 lib 参数。
    pub async fn audit(&self, kind: &str, payload: serde_json::Value) {
        sqlx::query(
            "INSERT INTO jobs (id, kind, payload, status, attempts, max_attempts, \
             progress, started_at, finished_at) \
             VALUES ($1, $2, $3, 'succeeded', 1, 1, $3, now(), now())",
        )
        .bind(Uuid::now_v7())
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await
        .ok();
    }
}

/// 版本快照裁剪（模块级：put_page 与 promote_page 共用；EN-59 提升可见性到 pub(crate)）。
pub(crate) async fn prune_page_versions(pool: &sqlx::PgPool, lib: Uuid, slug: &str, keep: i64) {
    sqlx::query(
        "DELETE FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 \
         AND id NOT IN ( \
         SELECT id FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 \
         ORDER BY created_at DESC, version DESC LIMIT $3)",
    )
    .bind(slug)
    .bind(lib)
    .bind(keep)
    .execute(pool)
    .await
    .ok();
}
