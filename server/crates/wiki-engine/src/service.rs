//! Wiki 服务层：页面 CRUD、图数据、ingest 入口、lint 调用。

use engram_jobs::{JobQueue, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::tsv_text;
use chrono::{DateTime, Utc};
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

#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
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
}

impl WikiService {
    pub fn new(pool: sqlx::PgPool, registry: ProviderRegistry) -> Self {
        Self {
            queue: JobQueue::new(pool.clone()),
            registry,
            pool,
        }
    }

    /// 触发两步 ingest（文本 + 标题）。sha 命中返回 true（跳过）。
    pub async fn ingest(&self, title: &str, text: &str) -> Result<bool, WikiError> {
        let (_, skipped) = ingest::enqueue_ingest(&self.queue, title, text).await?;
        Ok(skipped)
    }

    /// 从 knowledge 文档触发织入（upload 与 URL 通用，2026-09-04 补 URL 兜底）：
    /// raw_path 有 → 重新读取原文件并解析（保留原行为）；
    /// raw_path 空（URL 摄取）→ 用已分块文本按 seq 拼接——此前 URL 文档既不能
    /// --doc-id 手动织入（404）也不会被自动织入静默跳过，两路都收敛到 ingest(title, text)。
    pub async fn ingest_document(&self, doc_id: Uuid) -> Result<bool, WikiError> {
        let row: Option<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT title, raw_path, mime FROM documents WHERE id = $1")
                .bind(doc_id)
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
            // URL 摄取：无本地文件，用 chunks 表已解析文本按序拼接
            let chunks: Vec<String> = sqlx::query_scalar(
                "SELECT content FROM chunks WHERE document_id = $1 ORDER BY seq",
            )
            .bind(doc_id)
            .fetch_all(&self.pool)
            .await?;
            if chunks.is_empty() {
                return Err(WikiError::BadRequest(format!(
                    "文档 {doc_id} 无本地文件且无可织入的分块（可能尚未解析完成）"
                )));
            }
            chunks.join("\n\n")
        };
        self.ingest(&title, &text).await
    }

    pub async fn list_pages(
        &self,
        page_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        Ok(sqlx::query_as::<_, WikiPageDto>(
            "SELECT * FROM wiki_pages \
             WHERE ($1::text IS NULL OR page_type = $1) AND page_type NOT IN ('log') \
             ORDER BY updated_at DESC LIMIT $2",
        )
        .bind(page_type)
        .bind(limit.min(300))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_page(&self, slug: &str) -> Result<WikiPageDto, WikiError> {
        // 先精确匹配；未中则按「小写 + 空格转连字符」宽容重查——LLM 生成正文时
        // 常把双链写成标题原文（[[Rust 异步运行时]]），与真实 slug（rust-异步运行时）
        // 只差大小写和分隔符，精确匹配 404 后点过去就"没反应"。
        sqlx::query_as::<_, WikiPageDto>(
            "SELECT * FROM wiki_pages \
             WHERE slug = $1 OR slug = lower(replace($1, ' ', '-')) \
             ORDER BY (slug = $1) DESC LIMIT 1",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| WikiError::NotFound(format!("页面 {slug} 不存在")))
    }

    /// 人工编辑：origin=human、版本递增、重嵌入。folder 可选（None=保持原值/默认空）。
    /// via 可选（S-7）：执行者标记（如 "ai"）——落 frontmatter.via，区分真人编辑与 AI 代执行。
    pub async fn put_page(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        folder: Option<&str>,
        via: Option<&str>,
    ) -> Result<WikiPageDto, WikiError> {
        if !crate::markup::is_valid_slug(slug) {
            return Err(WikiError::BadRequest("slug 非法".into()));
        }
        // 新建时的 frontmatter：title/sources + via（若有）
        let mut fm_insert = serde_json::json!({"title": title, "sources": []});
        if let Some(v) = via {
            fm_insert["via"] = serde_json::json!(v);
        }
        // 更新时的 via 合并块：无 via 则空对象（保持原 frontmatter 不动）
        let fm_merge = via
            .map(|v| serde_json::json!({ "via": v }).to_string())
            .unwrap_or_else(|| "{}".into());
        let row = sqlx::query_as::<_, WikiPageDto>(
            "INSERT INTO wiki_pages (id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, 'concept', COALESCE($4, ''), $5, $6::jsonb, 'human', 1, to_tsvector('simple', $7)) \
             ON CONFLICT (slug) DO UPDATE SET \
                title = $3, content = $5, origin = 'human', \
                folder = COALESCE($4, wiki_pages.folder), \
                frontmatter = CASE WHEN $8::jsonb = '{}'::jsonb \
                    THEN wiki_pages.frontmatter \
                    ELSE wiki_pages.frontmatter || $8::jsonb END, \
                version = wiki_pages.version + 1, updated_at = now(), \
                tsv = to_tsvector('simple', $7) \
             RETURNING *",
        )
        .bind(Uuid::now_v7())
        .bind(slug)
        .bind(title)
        .bind(folder)
        .bind(content)
        .bind(fm_insert.to_string())
        .bind(tsv_text(content))
        .bind(fm_merge)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// 链接图（节点 = 页面，边 = wikilink；含 Louvain 社区 + 凝聚度）。
    pub async fn graph(&self) -> Result<GraphDto, WikiError> {
        let nodes: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type FROM wiki_pages",
        )
        .fetch_all(&self.pool)
        .await?;
        let edges: Vec<(String, String, f32)> =
            sqlx::query_as("SELECT from_slug, to_slug, weight FROM wiki_links")
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

    pub async fn lint(&self) -> Result<lint::LintReport, WikiError> {
        Ok(lint::lint(&self.pool).await?)
    }

    /// 提案合入（人审通过：把 job_events 里的 proposal 内容写入页面）。
    pub async fn apply_proposal(
        &self,
        slug: &str,
        content: &str,
        title: &str,
        via: Option<&str>,
    ) -> Result<WikiPageDto, WikiError> {
        // human 合入：保持 origin=human 语义（人确认的内容）；via 落 frontmatter 区分执行者
        self.put_page(slug, title, content, None, via).await
    }

    /// Wiki 检索（FTS + 向量 RRF 融合；W2：向量通道落地）。
    /// purpose 注入：检索走 LLM 时（AI 客户端读 query_context.purpose）提供方向意图——对齐 llm_wiki 的 query 注入。
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<WikiPageDto>, WikiError> {
        // K7：单字/纯标点无 token → 短路空结果（不再空跑 to_tsquery）
        if !engram_search::tokenize::has_query_tokens(query) {
            return Ok(vec![]);
        }
        let tsq = engram_search::tokenize::tsv_query_smart(query, 3);
        let limit = limit.min(50);

        // W2：查询向量（无 provider / 嵌入失败 → None → 纯 FTS）；L6：经记账门面
        let qv: Option<Vec<f32>> = self
            .registry
            .embed_for(Purpose::Embed, vec![query.to_string()], Some(1024), None)
            .await
            .ok()
            .and_then(|r| r.embeddings.first().cloned());

        if let Some(qv) = qv {
            // FTS + ANN 双候选 + RRF 融合（与 knowledge 同款模式）
            let rows: Vec<WikiPageDto> = sqlx::query_as(
                "WITH fts AS (SELECT slug, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
                 FROM wiki_pages, to_tsquery('simple', $1) q WHERE tsv @@ q LIMIT 100), \
                 vec AS (SELECT slug, ROW_NUMBER() OVER (ORDER BY embedding <=> $2) AS rank \
                 FROM wiki_pages WHERE embedding IS NOT NULL LIMIT 100) \
                 SELECT p.* FROM wiki_pages p \
                 LEFT JOIN fts ON fts.slug = p.slug \
                 LEFT JOIN vec ON vec.slug = p.slug \
                 WHERE fts.slug IS NOT NULL OR vec.slug IS NOT NULL \
                 ORDER BY (COALESCE(1.0/(60 + fts.rank), 0) + COALESCE(1.0/(60 + vec.rank), 0)) DESC \
                 LIMIT $3",
            )
            .bind(tsq)
            .bind(pgvector::Vector::from(qv))
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
            Ok(rows)
        } else {
            Ok(sqlx::query_as::<_, WikiPageDto>(
                "SELECT * FROM wiki_pages, to_tsquery('simple', $1) q \
                 WHERE tsv @@ q ORDER BY ts_rank(tsv, q) DESC LIMIT $2",
            )
            .bind(tsq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
        }
    }

    /// W2 存量补数：LLM 页 tsv 曾只嵌 slug——重写为 title+content 口径。
    /// 幂等（值不变不写）；jieba 分词必须经 Rust，故逐页计算。
    pub async fn backfill_tsv(&self) -> Result<u64, WikiError> {
        let pages: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages \
             WHERE origin = 'llm' AND page_type NOT IN ('index','log','overview')",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut n = 0u64;
        for (slug, title, content) in &pages {
            let text = format!("{title}\n{content}");
            let r = sqlx::query(
                "UPDATE wiki_pages SET tsv = to_tsvector('simple', $2) \
                 WHERE slug = $1 AND tsv IS DISTINCT FROM to_tsvector('simple', $2)",
            )
            .bind(slug)
            .bind(engram_search::tokenize::tsv_text(&text))
            .execute(&self.pool)
            .await?;
            n += r.rows_affected();
        }
        Ok(n)
    }

    /// 检索上下文包（query 时 purpose 注入的载体）：purpose + 命中页面，
    /// AI 客户端把 purpose 作为 system context 前缀使用。
    pub async fn search_with_purpose(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<serde_json::Value, WikiError> {
        let pages = self.search(query, limit).await?;
        // W-10（2026-09-04）：未设 purpose 时返回 null，与 GET /wiki/purpose 一致；
        // 不再用 purpose_context 的默认模板——「读当前设置」与「注入 LLM」语义分开。
        let purpose = crate::purpose::get_purpose(&self.pool).await.ok().flatten();
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

    // ---------- purpose（wiki 灵魂） ----------

    pub async fn get_purpose(&self) -> Result<Option<crate::purpose::Purpose>, WikiError> {
        crate::purpose::get_purpose(&self.pool)
            .await
            .map_err(WikiError::from)
    }

    pub async fn set_purpose(&self, p: &crate::purpose::Purpose) -> Result<(), WikiError> {
        crate::purpose::set_purpose(&self.pool, p)
            .await
            .map_err(WikiError::from)
    }

    // ---------- Review ----------

    pub async fn reviews(&self) -> Result<Vec<crate::review::ReviewItem>, WikiError> {
        crate::review::list_open(&self.pool)
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

    /// 检索结果/问答 → 直接落 queries 页型（人工归档）→ 同时入队再摄取吸收实体概念。
    pub async fn archive_query(
        &self,
        title: &str,
        question: &str,
        answer: &str,
    ) -> Result<bool, WikiError> {
        // W-13（2026-09-04）：同 title 已存档 → 幂等跳过（契约「重复→skipped」），
        // 不再落页 version+1 + 再摄取烧 LLM。
        let slug = format!("query-{title}");
        let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM wiki_pages WHERE slug = $1")
            .bind(&slug)
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
            "INSERT INTO wiki_pages (id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, 'queries', '查询', $4, $5, 'human', 1, to_tsvector('simple', $6)) \
             ON CONFLICT (slug) DO UPDATE SET content = $4, version = wiki_pages.version + 1, updated_at = now()",
        )
        .bind(Uuid::now_v7())
        .bind(&slug)
        .bind(title)
        .bind(&content)
        .bind(sqlx::types::Json(&fm))
        .bind(engram_search::tokenize::tsv_text(&content))
        .execute(&self.pool)
        .await?;

        // 2) 再摄取（实体概念网络吸收本次问答内容）
        let (_, skipped) = crate::ingest::enqueue_ingest(&self.queue, title, &content).await?;
        Ok(skipped)
    }

    // ---------- 级联删除 ----------

    pub async fn delete_source_cascade(
        &self,
        source_id: Uuid,
    ) -> Result<crate::cascade::CascadeReport, WikiError> {
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
        // 破坏性操作落审计行（与 memory 域「job 行即审计链」同哲学）——best-effort，不阻断返回
        self.audit(
            "wiki_source_cascade_delete",
            serde_json::json!({ "source_id": source_id, "report": report }),
        )
        .await;
        Ok(report)
    }

    // ---------- 图洞察 ----------

    pub async fn insights(&self) -> Result<crate::insights::InsightsReport, WikiError> {
        crate::insights::compute_insights(&self.pool)
            .await
            .map_err(WikiError::from)
    }

    pub async fn insight_dismiss(&self, key: &str) -> Result<(), WikiError> {
        crate::insights::dismiss(&self.pool, key)
            .await
            .map_err(WikiError::from)
    }

    pub async fn insight_reset(&self) -> Result<(), WikiError> {
        crate::insights::reset_dismissals(&self.pool)
            .await
            .map_err(WikiError::from)
    }

    /// 供 API 列出可删的 sources。
    pub async fn list_sources(
        &self,
    ) -> Result<Vec<(Uuid, Option<String>, String, String)>, WikiError> {
        Ok(sqlx::query_as(
            "SELECT id, title, status, sha256 FROM wiki_sources ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// 审计行（清空不吞审计凭证）：破坏性操作落 jobs 成功行，best-effort。
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
