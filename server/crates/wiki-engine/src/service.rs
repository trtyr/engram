//! Wiki 服务层：页面 CRUD、图数据、ingest 入口、lint 调用。
//! 多库（0037）：所有公开方法在 `&self` 后带 `lib: Uuid`，SQL 全部按库过滤/写入。

use chrono::{DateTime, Utc};

mod ingest_ops;
mod pages;
mod repair_ops;
mod search;

use engram_jobs::{JobQueue, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::{tsv_query_smart_wiki, tsv_text_wiki};
use sqlx::{FromRow, Row};
use uuid::Uuid;

use crate::ingest;
use crate::lint;

/// wiki_pages tsv 覆盖口径（EN-63）：slug + title + content 三合一——slug 复合词
/// 归一后（ai passthrough principle）与标题词都在索引里，搜索任何一段均可命中。
fn page_tsv_text(slug: &str, title: &str, content: &str) -> String {
    tsv_text_wiki(&format!("{slug} {title} {content}"))
}

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

/// 列表行（元数据，**不含正文**）——规模化 2026-09-20：万页下正文合计 18MB，目录树
/// 只需要元数据；正文走 `get_page`。`content_chars` 保留原 P1-7 语义（len 由 SQL
/// char_length 计算，不经网络），供调用方判断「值不值得拉全文」。
#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WikiPageMetaDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub page_type: String,
    /// 目录树层级（/ 分隔多级，Obsidian 式文件夹）
    pub folder: String,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub origin: String,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
    /// 正文字符数（SQL char_length，不含正文本体）
    pub content_chars: i32,
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

/// 规模化 task-3：查询类型路由——零成本规则分类决定 FTS/向量通道权重。
/// 判定顺序：对比 > 概念 > 场景 > 实体（默认）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    /// 对比题：「X vs Y」「X 和 Y 区别」「对比」——comparison 页天然匹配
    Comparison,
    /// 概念题：「什么是 X」「X 是什么」——概念页 + 摘要页
    Concept,
    /// 场景题：「怎么/如何/配置/部署」——语义相似优先（向量主导）
    Scenario,
    /// 实体题（默认）：专名/产品名/短词——精确匹配优先（FTS 主导）
    Entity,
}

impl QueryKind {
    /// RRF 通道权重：(fts, vec)。实体题 FTS 加倍（专名精确命中最准）；
    /// 场景题向量加倍（语义意图）；对比/概念默认等权。
    pub fn rrf_weights(&self) -> (f64, f64) {
        match self {
            QueryKind::Entity => (2.0, 1.0),
            QueryKind::Scenario => (1.0, 2.0),
            QueryKind::Concept | QueryKind::Comparison => (1.0, 1.0),
        }
    }
}

/// 纯规则分类（零成本——不加任何模型调用）。命中词表即归类，否则实体题兜底。
pub fn classify_query(query: &str) -> QueryKind {
    let q = query.to_lowercase();
    const COMPARE: [&str; 5] = ["vs", "对比", "区别", "相比", "差异"];
    const CONCEPT: [&str; 6] = ["什么是", "是什么", "什么叫", "概念", "含义", "介绍"];
    const SCENARIO: [&str; 8] = [
        "怎么", "如何", "怎样", "配置", "部署", "处理", "设置", "排查",
    ];
    if COMPARE.iter().any(|k| q.contains(k)) {
        return QueryKind::Comparison;
    }
    if CONCEPT.iter().any(|k| q.contains(k)) {
        return QueryKind::Concept;
    }
    if SCENARIO.iter().any(|k| q.contains(k)) {
        return QueryKind::Scenario;
    }
    QueryKind::Entity
}

/// 批次② 查询日志：检索后 UPSERT（calls 累计；零命中/低分计数——缺口挖掘原料）。
async fn log_query(
    pool: &sqlx::PgPool,
    lib: Uuid,
    query: &str,
    hits: usize,
    top_score: Option<f64>,
) -> Result<(), sqlx::Error> {
    maybe_cleanup_query_log(pool, lib).await;
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

/// 查询日志保留策略（规模化四件套 2026-09-20，goal mu9frlhh-4e1fmj task-2）：
/// 万级检索写入下行数受控——(1) TTL：90 天未活跃的查询行过期；
/// (2) 硬顶：每库最多保留 QUERY_LOG_MAX_ROWS 行，超出删最旧。
/// 触发：log_query 按天桶幂等（进程级，每天首次检索触发一次），
/// 后台 spawn 执行不阻塞 search 路径。
const QUERY_LOG_TTL_DAYS: i32 = 90;
const QUERY_LOG_MAX_ROWS: i64 = 5000;

fn query_log_day_bucket() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 / 86_400)
        .unwrap_or(0)
}

/// 进程级天桶去重：同一自然日只触发一次清理。
fn should_cleanup_query_log_today() -> bool {
    static LAST_DAY: std::sync::OnceLock<std::sync::atomic::AtomicI64> = std::sync::OnceLock::new();
    let last = LAST_DAY.get_or_init(|| std::sync::atomic::AtomicI64::new(-1));
    let today = query_log_day_bucket();
    last.swap(today, std::sync::atomic::Ordering::Relaxed) != today
}

/// 后台清理入口：fire-and-forget，失败仅日志（清理失败不影响检索）。
async fn maybe_cleanup_query_log(pool: &sqlx::PgPool, lib: Uuid) {
    if !should_cleanup_query_log_today() {
        return;
    }
    let pool = pool.clone();
    tokio::spawn(async move {
        match cleanup_query_log(&pool, lib).await {
            Ok(removed) if removed > 0 => {
                tracing::info!(removed, "wiki_query_log 例行清理完成（TTL + 行数上限）")
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "wiki_query_log 清理失败——留待明日重试"),
        }
    });
}

/// 查询日志清理：TTL 过期 + 行数硬顶（公开供测试与未来节律挂钩）。
pub async fn cleanup_query_log(pool: &sqlx::PgPool, lib: Uuid) -> Result<u64, sqlx::Error> {
    let mut removed = sqlx::query(
        "DELETE FROM wiki_query_log \
         WHERE library_id = $1 \
           AND last_queried_at < now() - ($2 || ' days')::interval",
    )
    .bind(lib)
    .bind(QUERY_LOG_TTL_DAYS.to_string())
    .execute(pool)
    .await?
    .rows_affected();
    // 硬顶：保留最近 QUERY_LOG_MAX_ROWS 行，更旧的淘汰（长尾查询防膨胀）
    removed += sqlx::query(
        "DELETE FROM wiki_query_log \
         WHERE id IN (\
           SELECT id FROM wiki_query_log WHERE library_id = $1 \
           ORDER BY last_queried_at DESC OFFSET $2\
         )",
    )
    .bind(lib)
    .bind(QUERY_LOG_MAX_ROWS)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(removed)
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
    /// 目录树层级（规模化 task-5：子图过滤维度之一）
    pub folder: String,
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
