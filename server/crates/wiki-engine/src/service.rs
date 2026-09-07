//! Wiki 服务层：页面 CRUD、图数据、ingest 入口、lint 调用。

use chrono::{DateTime, Utc};
use engram_jobs::{JobQueue, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::tsv_text;
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

    /// 触发两步 ingest（文本 + 标题）。返回三态（D27）：已就绪跳过 / 在途 / 新入队。
    /// D24：空标题/空文本响亮拒绝（空文本任务曾在队列里滞留不执行、空标题白烧一次 LLM）。
    pub async fn ingest(
        &self,
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
        Ok(ingest::enqueue_ingest(&self.queue, title, text).await?)
    }

    /// 从 wiki 文档触发织入（upload 与 URL 通用，2026-09-04 补 URL 兜底）：
    /// raw_path 有 → 重新读取原文件并解析（保留原行为）；
    /// raw_path 空（URL 摄取）→ 用已分块文本按 seq 拼接——此前 URL 文档既不能
    /// --doc-id 手动织入（404）也不会被自动织入静默跳过，两路都收敛到 ingest(title, text)。
    pub async fn ingest_document(
        &self,
        doc_id: Uuid,
    ) -> Result<crate::ingest::IngestOutcome, WikiError> {
        let row: Option<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT title, raw_path, mime FROM wiki_documents WHERE id = $1")
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
                "SELECT content FROM wiki_chunks WHERE document_id = $1 ORDER BY seq",
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

    /// 页面列表（D28 keyset 分页，单页上限 300）：cursor = 上一页最后一条的
    /// `{updated_at ISO8601}|{id}`，首查不传。ORDER BY 带 id 决稳——
    /// 此前静默截断曾让最老的页面从列表「消失」（graph/lint 却可见）。
    pub async fn list_pages(
        &self,
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
                     WHERE ($1::text IS NULL OR page_type = $1) AND page_type NOT IN ('log') \
                     ORDER BY updated_at DESC, id DESC LIMIT $2",
            )
            .bind(page_type)
            .bind(limit.min(300))
            .fetch_all(&self.pool)
            .await?),
            Some(raw) => {
                let (ts, id) = parse_cursor(raw)?;
                Ok(sqlx::query_as::<_, WikiPageDto>(
                    "SELECT * FROM wiki_pages \
                     WHERE ($1::text IS NULL OR page_type = $1) AND page_type NOT IN ('log') \
                       AND (updated_at, id) < ($3::timestamptz, $4::uuid) \
                     ORDER BY updated_at DESC, id DESC LIMIT $2",
                )
                .bind(page_type)
                .bind(limit.min(300))
                .bind(ts)
                .bind(id)
                .fetch_all(&self.pool)
                .await?)
            }
        }
    }

    pub async fn get_page(&self, slug: &str) -> Result<WikiPageDto, WikiError> {
        // 先精确匹配；未中则按「小写 + 空格转连字符」宽容重查——LLM 生成正文时
        // 常把双链写成标题原文（[[Rust 异步运行时]]），与真实 slug（rust-异步运行时）
        // 只差大小写和分隔符，精确匹配 404 后点过去就"没反应"。
        // R 报告 P1-11 双寻址：slug 未中再按 title 精确兜底（标题寻址）。
        sqlx::query_as::<_, WikiPageDto>(
            "SELECT * FROM wiki_pages \
             WHERE slug = $1 OR slug = lower(replace($1, ' ', '-')) OR title = $1 \
             ORDER BY (slug = $1) DESC, (title = $1) DESC LIMIT 1",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| WikiError::NotFound(format!("页面 {slug} 不存在")))
    }

    /// slug/title 宽容解析成真实 slug（删除/版本操作用，与 get_page 同一匹配口径）。
    async fn resolve_slug(&self, slug_or_title: &str) -> Result<String, WikiError> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT slug FROM wiki_pages \
             WHERE slug = $1 OR slug = lower(replace($1, ' ', '-')) OR title = $1 \
             ORDER BY (slug = $1) DESC LIMIT 1",
        )
        .bind(slug_or_title)
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| WikiError::NotFound(format!("页面 {slug_or_title} 不存在")))
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
            "INSERT INTO wiki_page_versions (id, slug, version, title, page_type, folder, content, origin) \
             SELECT $1, slug, version, title, page_type, folder, content, origin \
             FROM wiki_pages WHERE slug = $2",
        )
        .bind(Uuid::now_v7())
        .bind(slug)
        .execute(&self.pool)
        .await?;
        self.prune_page_versions(slug).await;
        let row = sqlx::query_as::<_, WikiPageDto>(
            "INSERT INTO wiki_pages (id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
             VALUES ($1, $2, $3, 'concept', COALESCE($4, ''), $5, $6::jsonb, 'human', 1, to_tsvector('simple', $7)) \
             ON CONFLICT (slug) DO UPDATE SET \
                title = $3, content = $5, origin = 'human', \
                folder = COALESCE($4, wiki_pages.folder), \
                frontmatter = wiki_pages.frontmatter || $8::jsonb, \
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

        // D4：落页后重算本页 wikilinks——graph/孤页检测与 lint 同源（此前
        // put_page 不写 wiki_links，AI 写页的互链对图与 lint 不可见）
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1")
            .bind(slug)
            .execute(&self.pool)
            .await?;
        for target in crate::markup::extract_wikilinks(content) {
            sqlx::query(
                "INSERT INTO wiki_links (from_slug, to_slug, weight) VALUES ($1, $2, 3.0) \
                 ON CONFLICT (from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(&target)
            .execute(&self.pool)
            .await
            .ok();
        }
        Ok(row)
    }

    /// 链接图（节点 = 页面，边 = wikilink；含 Louvain 社区 + 凝聚度）。
    pub async fn graph(&self) -> Result<GraphDto, WikiError> {
        // D23：排除系统 log 页（list_pages 不可见，图里也不该出现——否则节点无法溯源）
        let nodes: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type FROM wiki_pages \
             WHERE page_type <> 'log'",
        )
        .fetch_all(&self.pool)
        .await?;
        // 边随节点过滤：任一端是 log 页的边一并剔除（防悬空引用进社区发现）
        let edges: Vec<(String, String, f32)> = sqlx::query_as(
            "SELECT l.from_slug, l.to_slug, l.weight FROM wiki_links l \
             WHERE EXISTS (SELECT 1 FROM wiki_pages f WHERE f.slug = l.from_slug AND f.page_type <> 'log') \
               AND EXISTS (SELECT 1 FROM wiki_pages t WHERE t.slug = l.to_slug AND t.page_type <> 'log')",
        )
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
            // FTS + ANN 双候选 + RRF 融合（与 wiki 文档同款模式）
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
        // D5：slug 或 title 任一命中即幂等跳过（此前仅 slug 检查，title 尾随差异漏网 → 覆盖旧答案）
        let exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM wiki_pages WHERE slug = $1 OR title = $2")
                .bind(&slug)
                .bind(title)
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
             ON CONFLICT (slug) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(&slug)
        .bind(title)
        .bind(&content)
        .bind(sqlx::types::Json(&fm))
        .bind(engram_search::tokenize::tsv_text(&content))
        .execute(&self.pool)
        .await?;

        // 2) 再摄取（实体概念网络吸收本次问答内容）——D27 三态：仅已就绪算 skipped
        let outcome = crate::ingest::enqueue_ingest(&self.queue, title, &content).await?;
        Ok(outcome.skipped())
    }

    /// 存量回填（D4 遗留）：重析全部页面正文重建 wiki_links。
    /// 修复前写入的页面链接索引缺失——一次性全量重析（幂等，先清后建）。
    pub async fn rebuild_all_links(&self) -> Result<u64, WikiError> {
        let pages: Vec<(String, String)> =
            sqlx::query_as("SELECT slug, content FROM wiki_pages ORDER BY slug")
                .fetch_all(&self.pool)
                .await?;
        sqlx::query("DELETE FROM wiki_links")
            .execute(&self.pool)
            .await?;
        let mut n = 0;
        for (slug, content) in &pages {
            for target in crate::markup::extract_wikilinks(content) {
                sqlx::query(
                    "INSERT INTO wiki_links (from_slug, to_slug, weight) VALUES ($1, $2, 3.0)                      ON CONFLICT (from_slug, to_slug) DO NOTHING",
                )
                .bind(slug)
                .bind(&target)
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
    /// （R 报告「删除抹掉全部历史」的回收通道；版本历史本身保留）。
    pub async fn delete_page(&self, slug: &str) -> Result<bool, WikiError> {
        let slug = self.resolve_slug(slug).await?;
        sqlx::query(
            "INSERT INTO wiki_page_versions (id, slug, version, title, page_type, folder, content, origin) \
             SELECT $1, slug, version, title, page_type, folder, content, origin \
             FROM wiki_pages WHERE slug = $2",
        )
        .bind(Uuid::now_v7())
        .bind(&slug)
        .execute(&self.pool)
        .await?;
        self.prune_page_versions(&slug).await;
        let n = sqlx::query("DELETE FROM wiki_pages WHERE slug = $1")
            .bind(&slug)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(WikiError::NotFound(format!("页面 {slug} 不存在")));
        }
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1 OR to_slug = $1")
            .bind(&slug)
            .execute(&self.pool)
            .await?;
        Ok(true)
    }

    // ---------- 版本历史（R 报告建议 #5：列表 + 回滚） ----------

    /// 每 slug 保留的版本快照上限（与 skill_revisions 同口径）。
    const VERSION_KEEP: i64 = 50;

    /// 裁剪旧快照（每 slug 只留最近 VERSION_KEEP 条；best-effort，不影响主流程）。
    async fn prune_page_versions(&self, slug: &str) {
        sqlx::query(
            "DELETE FROM wiki_page_versions WHERE slug = $1 AND id NOT IN ( \
             SELECT id FROM wiki_page_versions WHERE slug = $1 \
             ORDER BY created_at DESC, version DESC LIMIT $2)",
        )
        .bind(slug)
        .bind(Self::VERSION_KEEP)
        .execute(&self.pool)
        .await
        .ok();
    }

    /// 页面版本列表（新→旧；不带正文，content_chars 供决策）。
    /// 已删除的页面按 slug 直查快照表——恢复通道不因页面不在而 404。
    pub async fn page_versions(&self, slug: &str) -> Result<Vec<WikiPageVersionDto>, WikiError> {
        let slug = match self.resolve_slug(slug).await {
            Ok(s) => s,
            Err(WikiError::NotFound(_)) => slug.to_string(),
            Err(e) => return Err(e),
        };
        Ok(sqlx::query_as::<_, WikiPageVersionDto>(
            "SELECT id, slug, version, title, page_type, folder, origin, \
             length(content)::bigint AS content_chars, created_at \
             FROM wiki_page_versions WHERE slug = $1 ORDER BY version DESC, created_at DESC",
        )
        .bind(&slug)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 读取一个版本快照的正文（回滚前预览用）。已删除页面按 slug 直查。
    pub async fn page_version_content(
        &self,
        slug: &str,
        version: i32,
    ) -> Result<String, WikiError> {
        let slug = match self.resolve_slug(slug).await {
            Ok(s) => s,
            Err(WikiError::NotFound(_)) => slug.to_string(),
            Err(e) => return Err(e),
        };
        let row: Option<String> = sqlx::query_scalar(
            "SELECT content FROM wiki_page_versions WHERE slug = $1 AND version = $2",
        )
        .bind(&slug)
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
    /// 页面已被删除时从快照重建（沿用页型/目录，版本号接续快照史）。
    pub async fn restore_page_version(
        &self,
        slug: &str,
        version: i32,
    ) -> Result<WikiPageDto, WikiError> {
        let snap: (String, String, String, String, i32) = sqlx::query_as(
            "SELECT title, content, page_type, folder, version \
             FROM wiki_page_versions WHERE slug = $1 AND version = $2",
        )
        .bind(slug)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            WikiError::NotFound(format!(
                "没有 {slug}#{version} 的快照——先查 versions 列表取可用版本号"
            ))
        })?;
        let (title, content, page_type, folder, _) = snap;
        match self.get_page(slug).await {
            Ok(_) => {
                // 活页：走 put_page（快照现状 → 落目标内容 → 版本 +1、重算链接）
                self.put_page(slug, &title, &content, Some(&folder), Some("restore"))
                    .await
            }
            Err(WikiError::NotFound(_)) => {
                // 死页重建：版本号接续快照史（避免清零后与历史快照版本撞号）
                let next: i32 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(version), 0) + 1 \
                     FROM (SELECT version FROM wiki_page_versions WHERE slug = $1 \
                           UNION ALL SELECT version FROM wiki_pages WHERE slug = $1) t",
                )
                .bind(slug)
                .fetch_one(&self.pool)
                .await?;
                let fm = serde_json::json!({"title": title, "sources": [], "via": "restore"});
                let row = sqlx::query_as::<_, WikiPageDto>(
                    "INSERT INTO wiki_pages (id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, 'human', $8, to_tsvector('simple', $9)) \
                     RETURNING *",
                )
                .bind(Uuid::now_v7())
                .bind(slug)
                .bind(&title)
                .bind(&page_type)
                .bind(&folder)
                .bind(&content)
                .bind(fm.to_string())
                .bind(next)
                .bind(tsv_text(&content))
                .fetch_one(&self.pool)
                .await?;
                for target in crate::markup::extract_wikilinks(&content) {
                    sqlx::query(
                        "INSERT INTO wiki_links (from_slug, to_slug, weight) VALUES ($1, $2, 3.0) \
                         ON CONFLICT (from_slug, to_slug) DO NOTHING",
                    )
                    .bind(slug)
                    .bind(&target)
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
