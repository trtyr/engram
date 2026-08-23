//! Wiki 服务层：页面 CRUD、图数据、ingest 入口、lint 调用。

use agent_memory_jobs::{JobQueue, JobTemplate};
use agent_memory_search::tokenize::tsv_text;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::ingest;
use crate::lint;

/// LLM 引用（ingest job 注入）。
pub type LlmRef = std::sync::Arc<dyn agent_memory_distill::llm_port::DistillLlm>;

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

impl From<agent_memory_jobs::types::JobError> for WikiError {
    fn from(e: agent_memory_jobs::types::JobError) -> Self {
        WikiError::Storage(e.to_string())
    }
}

#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WikiPageDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub page_type: String,
    pub content: String,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub origin: String,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
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
    #[serde(default)]
    pub size: usize,
    pub cohesion: f64,
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
}

impl WikiService {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            queue: JobQueue::new(pool.clone()),
            pool,
        }
    }

    /// 触发两步 ingest（文本 + 标题）。sha 命中返回 true（跳过）。
    pub async fn ingest(&self, title: &str, text: &str) -> Result<bool, WikiError> {
        let (_, skipped) = ingest::enqueue_ingest(&self.queue, title, text).await?;
        Ok(skipped)
    }

    /// 从 knowledge 文档触发（读取已解析文本：重新读取原文并解析）。
    pub async fn ingest_knowledge_document(
        &self,
        doc_id: Uuid,
        raw: &[u8],
        name: &str,
        content_type: Option<&str>,
    ) -> Result<bool, WikiError> {
        let text = agent_memory_parsing::parse_bytes(name, content_type, raw)
            .map_err(|e| WikiError::BadRequest(e.to_string()))?;
        let title: Option<String> = sqlx::query_scalar("SELECT title FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
        self.ingest(title.as_deref().unwrap_or(name), &text).await
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
        sqlx::query_as::<_, WikiPageDto>("SELECT * FROM wiki_pages WHERE slug = $1")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| WikiError::NotFound(format!("页面 {slug} 不存在")))
    }

    /// 人工编辑：origin=human、版本递增、重嵌入。
    pub async fn put_page(
        &self,
        slug: &str,
        title: &str,
        content: &str,
    ) -> Result<WikiPageDto, WikiError> {
        if !crate::markup::is_valid_slug(slug) {
            return Err(WikiError::BadRequest("slug 非法".into()));
        }
        let row = sqlx::query_as::<_, WikiPageDto>(
            "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, 'concept', $4, $5::jsonb, 'human', 1, to_tsvector('simple', $6)) \
             ON CONFLICT (slug) DO UPDATE SET \
                title = $3, content = $4, origin = 'human', \
                version = wiki_pages.version + 1, updated_at = now(), \
                tsv = to_tsvector('simple', $6) \
             RETURNING *",
        )
        .bind(Uuid::now_v7())
        .bind(slug)
        .bind(title)
        .bind(content)
        .bind(serde_json::json!({"title": title, "sources": []}))
        .bind(tsv_text(content))
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
        Ok(GraphDto {
            communities: cohesion
                .into_iter()
                .map(|(id, cohesion)| CommunityInfo {
                    id,
                    size: 0,
                    cohesion,
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
    ) -> Result<WikiPageDto, WikiError> {
        // human 合入：保持 origin=human 语义（人确认的内容）
        self.put_page(slug, title, content).await
    }

    /// Wiki 检索（FTS + 向量 RRF）。purpose 注入：检索走 LLM 时（AI 客户端
    /// 读 query_context.purpose）提供方向意图——对齐 llm_wiki 的 query 注入。
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<WikiPageDto>, WikiError> {
        let tsq = agent_memory_search::tokenize::tsv_query(query);
        Ok(sqlx::query_as::<_, WikiPageDto>(
            "SELECT * FROM wiki_pages, to_tsquery('simple', $1) q \
             WHERE tsv @@ q ORDER BY ts_rank(tsv, q) DESC LIMIT $2",
        )
        .bind(tsq)
        .bind(limit.min(50))
        .fetch_all(&self.pool)
        .await?)
    }

    /// 检索上下文包（query 时 purpose 注入的载体）：purpose + 命中页面，
    /// AI 客户端把 purpose 作为 system context 前缀使用。
    pub async fn search_with_purpose(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<serde_json::Value, WikiError> {
        let pages = self.search(query, limit).await?;
        let purpose = crate::purpose::purpose_context(&self.pool).await;
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
        crate::review::resolve(&self.pool, id, action, dismiss)
            .await
            .map_err(WikiError::from)
    }

    // ---------- queries 页型闭环 ----------

    /// 检索结果/问答 → 直接落 queries 页型（人工归档）→ 同时入队再摄取吸收实体概念。
    pub async fn archive_query(
        &self,
        title: &str,
        question: &str,
        answer: &str,
    ) -> Result<bool, WikiError> {
        let ts = chrono::Utc::now().format("%Y-%m-%d");
        let content = format!(
            "# {title}\n\n**问**：{question}\n\n**答**：{answer}\n\n（来源：检索存档 {ts}）"
        );

        // 1) 直接落 queries 页（page_type=queries，origin=human——人触发的存档）
        let slug = format!("query-{title}");
        let fm = serde_json::json!({"title": title, "page_type": "queries", "sources": []});
        sqlx::query(
            "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, 'queries', $4, $5, 'human', 1, to_tsvector('simple', $6)) \
             ON CONFLICT (slug) DO UPDATE SET content = $4, version = wiki_pages.version + 1, updated_at = now()",
        )
        .bind(Uuid::now_v7())
        .bind(&slug)
        .bind(title)
        .bind(&content)
        .bind(sqlx::types::Json(&fm))
        .bind(agent_memory_search::tokenize::tsv_text(&content))
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
        crate::cascade::cascade_delete_source(&self.pool, source_id)
            .await
            .map_err(WikiError::from)
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
}
