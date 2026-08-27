//! 记忆域服务：L0 写入/触发、检索、上下文包、L1 治理、L3 画像视图。

use agent_memory_jobs::types::Job;
use agent_memory_jobs::{JobQueue, JobTemplate};
use agent_memory_llm::ProviderRegistry;
use agent_memory_llm::provider::LlmProvider as _;
use agent_memory_llm::types::{EmbedRequest, Purpose};
use agent_memory_search::tokenize::tokenize;
use agent_memory_search::{SearchHit, search_atoms, search_scenarios};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// 记忆域错误（api 层转 ApiError）。
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
    #[error("LLM 未配置（检索退化为全文通道）: {0}")]
    LlmNotConfigured(String),
}

impl From<sqlx::Error> for MemoryError {
    fn from(e: sqlx::Error) -> Self {
        MemoryError::Storage(e.to_string())
    }
}

// ---------- DTO（api 直接复用，utoipa schema） ----------

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SessionDto {
    pub id: Uuid,
    pub agent: String,
    #[schema(value_type = Object)]
    pub content: serde_json::Value,
    pub distill_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AtomDto {
    pub id: Uuid,
    pub kind: String,
    pub content: String,
    pub confidence: f32,
    pub status: String,
    pub superseded_by: Option<Uuid>,
    pub needs_review: bool,
    pub hit_count: i32,
    pub scenario_id: Option<Uuid>,
    #[schema(value_type = Object)]
    pub source_refs: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScenarioDto {
    pub id: Uuid,
    pub topic: String,
    pub summary: String,
    #[schema(value_type = Object)]
    pub atom_refs: serde_json::Value,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct PersonaVersion {
    pub id: Uuid,
    pub aspect: String,
    pub content: String,
    #[schema(value_type = Object)]
    pub evidence_refs: serde_json::Value,
    pub version: i32,
    pub prompt_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContextPack {
    /// L3：画像分面（当前版本，全量）
    pub persona: Vec<PersonaVersion>,
    /// L2：相关/最近场景
    pub scenarios: Vec<ScenarioDto>,
    /// L1：补充原子
    pub atoms: Vec<AtomDto>,
    pub meta: ContextMeta,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContextMeta {
    pub chars_used: usize,
    pub truncated: bool,
    pub query: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SearchResponse {
    pub l1: Vec<SearchHit>,
    pub l2: Vec<SearchHit>,
    pub l3: Vec<PersonaVersion>,
    pub query: String,
}

// ---------- 服务 ----------

#[derive(Clone)]
pub struct MemoryService {
    pool: PgPool,
    queue: JobQueue,
    registry: ProviderRegistry,
    /// 防抖窗口（秒）
    pub debounce_secs: i64,
}

impl MemoryService {
    pub fn new(pool: PgPool, registry: ProviderRegistry) -> Self {
        Self {
            queue: JobQueue::new(pool.clone()),
            pool,
            registry,
            debounce_secs: 30,
        }
    }

    // ---------- L0 ----------

    /// 写 L0 会话并按策略触发蒸馏。
    pub async fn write_session(
        &self,
        agent: &str,
        turns: serde_json::Value,
        distill: &str,
    ) -> Result<SessionDto, MemoryError> {
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("会话至少一轮".into()));
        }
        let id = Uuid::now_v7();
        let row = sqlx::query_as::<_, SessionDto>(
            "INSERT INTO raw_sessions (id, agent, content) VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(id)
        .bind(agent)
        .bind(sqlx::types::Json(&turns))
        .fetch_one(&self.pool)
        .await?;

        match distill {
            "auto" => {
                agent_memory_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                    .await
                    .ok();
            }
            "manual" => {
                self.queue
                    .enqueue(
                        JobTemplate::new("extract_atoms").with_payload(json!({"reason": "manual"})),
                    )
                    .await
                    .ok();
            }
            _ => {}
        }
        Ok(row)
    }

    pub async fn list_sessions(
        &self,
        agent: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<SessionDto>, MemoryError> {
        Ok(sqlx::query_as::<_, SessionDto>(
            "SELECT * FROM raw_sessions \
             WHERE ($1::text IS NULL OR agent = $1) AND ($2::timestamptz IS NULL OR created_at < $2) \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(agent)
        .bind(cursor)
        .bind(limit.min(200))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        sqlx::query_as::<_, SessionDto>("SELECT * FROM raw_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))
    }

    /// L0 擦除：删会话 + 引用它的 atoms 标记来源失效。
    pub async fn erase_session(&self, id: Uuid) -> Result<(), MemoryError> {
        let affected = sqlx::query("DELETE FROM raw_sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(MemoryError::NotFound(format!("会话 {id} 不存在")));
        }
        // 来源失效标记：source_refs 里含该会话的原子加 erased 标记
        let atoms: Vec<(Uuid, serde_json::Value)> =
            sqlx::query_as("SELECT id, source_refs FROM atoms WHERE source_refs::text LIKE $1")
                .bind(format!("%{id}%"))
                .fetch_all(&self.pool)
                .await?;
        for (aid, refs) in atoms {
            let marked = mark_erased(refs, id);
            sqlx::query("UPDATE atoms SET source_refs = $2, updated_at = now() WHERE id = $1")
                .bind(aid)
                .bind(sqlx::types::Json(&marked))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn trigger_distill(&self, full: bool) -> Result<Vec<Job>, MemoryError> {
        agent_memory_distill::chain::trigger_manual(&self.queue, full)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))
    }

    // ---------- L1 ----------

    pub async fn list_atoms(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
        needs_review: Option<bool>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<AtomDto>, MemoryError> {
        Ok(sqlx::query_as::<_, AtomDto>(
            "SELECT * FROM atoms \
             WHERE ($1::text IS NULL OR kind = $1) AND ($2::text IS NULL OR status = $2) \
               AND ($3::bool IS NULL OR needs_review = $3) \
               AND ($4::timestamptz IS NULL OR created_at < $4) \
             ORDER BY created_at DESC LIMIT $5",
        )
        .bind(kind)
        .bind(status)
        .bind(needs_review)
        .bind(cursor)
        .bind(limit.min(500))
        .fetch_all(&self.pool)
        .await?)
    }

    /// 手工新增（人审补充；active 直接入库）。
    pub async fn create_atom(
        &self,
        kind: &str,
        content: &str,
        confidence: f32,
    ) -> Result<AtomDto, MemoryError> {
        let id = Uuid::now_v7();
        let emb = self.try_embed(&[content.to_string()]).await;
        let row = sqlx::query_as::<_, AtomDto>(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, embedding, tsv) \
             VALUES ($1, $2, $3, $4, 'active', '[]'::jsonb, $5, to_tsvector('simple', $6)) RETURNING *",
        )
        .bind(id)
        .bind(kind)
        .bind(content)
        .bind(confidence)
        .bind(emb.as_ref().and_then(|v| v.first()).map(|v| pgvector::Vector::from(v.clone())))
        .bind(agent_memory_search::tokenize::tsv_text(content))
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn update_atom(
        &self,
        id: Uuid,
        content: Option<&str>,
        confidence: Option<f32>,
        status: Option<&str>,
    ) -> Result<AtomDto, MemoryError> {
        let cur = sqlx::query_as::<_, AtomDto>("SELECT * FROM atoms WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("原子 {id} 不存在")))?;

        let new_content = content.unwrap_or(&cur.content).to_string();
        let new_conf = confidence.unwrap_or(cur.confidence);
        let new_status = match status {
            Some("archived") => "archived",
            Some("active") if cur.status == "archived" => "active",
            Some("active") | Some("superseded") | Some("candidate") => {
                return Err(MemoryError::BadRequest(
                    "status 只允许 active/archived 切换；supersede 走矛盾流程".into(),
                ));
            }
            _ => cur.status.as_str(),
        };
        let content_changed = new_content != cur.content;
        let emb = if content_changed {
            self.try_embed(std::slice::from_ref(&new_content)).await
        } else {
            None
        };

        let row = sqlx::query_as::<_, AtomDto>(
            "UPDATE atoms SET content = $2, confidence = $3, status = $4, \
                 embedding = COALESCE($5, embedding), tsv = to_tsvector('simple', $6), updated_at = now() \
             WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(&new_content)
        .bind(new_conf)
        .bind(new_status)
        .bind(emb.as_ref().and_then(|v| v.first()).map(|v| pgvector::Vector::from(v.clone())))
        .bind(agent_memory_search::tokenize::tsv_text(&new_content))
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    // ---------- L2 / L3 ----------

    pub async fn list_scenarios(&self, limit: i64) -> Result<Vec<ScenarioDto>, MemoryError> {
        Ok(sqlx::query_as::<_, ScenarioDto>(
            "SELECT * FROM scenarios ORDER BY updated_at DESC LIMIT $1",
        )
        .bind(limit.min(200))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_scenario(&self, id: Uuid) -> Result<ScenarioDto, MemoryError> {
        sqlx::query_as::<_, ScenarioDto>("SELECT * FROM scenarios WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("场景 {id} 不存在")))
    }

    /// 当前画像（每分面最新版）。
    pub async fn persona(&self) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(sqlx::query_as::<_, PersonaVersion>(
            "SELECT DISTINCT ON (aspect) * FROM persona_aspects ORDER BY aspect, version DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// 分面版本历史。
    pub async fn persona_history(&self, aspect: &str) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(sqlx::query_as::<_, PersonaVersion>(
            "SELECT * FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC",
        )
        .bind(aspect)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 回滚分面到历史版本（以新版本号落地当前内容——历史不可变）。
    pub async fn persona_rollback(
        &self,
        aspect: &str,
        to_version: i32,
    ) -> Result<PersonaVersion, MemoryError> {
        let target = sqlx::query_as::<_, PersonaVersion>(
            "SELECT * FROM persona_aspects WHERE aspect = $1 AND version = $2",
        )
        .bind(aspect)
        .bind(to_version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| MemoryError::NotFound(format!("版本 {aspect}#{to_version} 不存在")))?;

        let cur: Option<Option<i32>> =
            sqlx::query_scalar("SELECT MAX(version) FROM persona_aspects WHERE aspect = $1")
                .bind(aspect)
                .fetch_optional(&self.pool)
                .await?;
        let next_v = cur.flatten().map(|v| v + 1).unwrap_or(1);

        sqlx::query(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
             VALUES ($1, $2, $3, $4::jsonb, $5, 'rollback')",
        )
        .bind(Uuid::now_v7())
        .bind(aspect)
        .bind(&target.content)
        .bind(sqlx::types::Json(&json!({"rollback_to": to_version})))
        .bind(next_v)
        .execute(&self.pool)
        .await?;
        self.persona_history(aspect)
            .await
            .map(|mut v| v.swap_remove(0))
    }

    // ---------- 检索 ----------

    async fn try_embed(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        match self.registry.resolve(Purpose::Embed).await {
            Ok((provider, model)) => provider
                .embed(EmbedRequest {
                    model,
                    inputs: texts.to_vec(),
                    dimensions: Some(1024),
                })
                .await
                .ok()
                .map(|r| r.embeddings),
            Err(_) => None,
        }
    }

    /// B9 命中反馈：检索命中即异步回写 hit_count（best-effort，失败只记日志）。
    /// 不刷 updated_at——hit 是使用热度而非内容变化，避免扰动「最近更新」排序。
    fn fire_hit_feedback(&self, table: &'static str, ids: Vec<Uuid>) {
        if ids.is_empty() {
            return;
        }
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let sql = if table == "atoms" {
                "UPDATE atoms SET hit_count = hit_count + 1 WHERE id = ANY($1)"
            } else {
                "UPDATE scenarios SET hit_count = hit_count + 1 WHERE id = ANY($1)"
            };
            if let Err(e) = sqlx::query(sql).bind(&ids).execute(&pool).await {
                tracing::warn!(error = %e, table, "hit_count 回写失败（不影响检索结果）");
            }
        });
    }

    /// 分层检索。无 embedding 通道时自动退化为纯 FTS。
    pub async fn search(
        &self,
        query: &str,
        layers: &[&str],
        max_items: i64,
    ) -> Result<SearchResponse, MemoryError> {
        let qv = self
            .try_embed(&[query.to_string()])
            .await
            .and_then(|v| v.first().cloned());
        let want_l1 = layers.is_empty() || layers.contains(&"l1");
        let want_l2 = layers.is_empty() || layers.contains(&"l2");
        let want_l3 = layers.is_empty() || layers.contains(&"l3");

        let l1 = if want_l1 {
            search_atoms(&self.pool, query, qv.as_deref(), max_items).await?
        } else {
            vec![]
        };
        let l2 = if want_l2 {
            search_scenarios(&self.pool, query, qv.as_deref(), max_items).await?
        } else {
            vec![]
        };
        // L3：小体量，token 命中过滤
        let l3 = if want_l3 {
            let tokens: std::collections::HashSet<String> = tokenize(query).into_iter().collect();
            self.persona()
                .await?
                .into_iter()
                .filter(|p| tokens.iter().any(|t| p.content.contains(t.as_str())))
                .collect()
        } else {
            vec![]
        };
        // B9：命中反馈（异步 best-effort，不阻塞返回）
        self.fire_hit_feedback("atoms", l1.iter().map(|h| h.id).collect());
        self.fire_hit_feedback("scenarios", l2.iter().map(|h| h.id).collect());
        Ok(SearchResponse {
            l1,
            l2,
            l3,
            query: query.to_string(),
        })
    }

    /// 冷启动上下文包：L3 全量 + L2 相关/最近 + L1 补充，预算裁剪。
    pub async fn context_pack(
        &self,
        query: Option<&str>,
        budget_items: usize,
        budget_chars: usize,
    ) -> Result<ContextPack, MemoryError> {
        let mut chars_used = 0usize;
        let mut truncated = false;

        let count = |s: &str, used: &mut usize, trunc: &mut bool| -> bool {
            if *used + s.len() > budget_chars {
                *trunc = true;
                false
            } else {
                *used += s.len();
                true
            }
        };

        // 有 query 时预计算 query 向量（L2/L1 共用，避免重复 embed）
        let qv: Option<Vec<f32>> = match query {
            Some(q) => self
                .try_embed(&[q.to_string()])
                .await
                .and_then(|v| v.first().cloned()),
            None => None,
        };

        // L3 全量（很小）
        let persona: Vec<PersonaVersion> = self
            .persona()
            .await?
            .into_iter()
            .take_while(|p| count(&p.content, &mut chars_used, &mut truncated))
            .collect();

        // L2：有 query 按相关性，否则最近
        let scenarios = if let Some(q) = query {
            search_scenarios(&self.pool, q, qv.as_deref(), (budget_items as i64).max(3)).await?
        } else {
            self.list_scenarios((budget_items as i64).max(3) / 2)
                .await?
                .into_iter()
                .map(|s| SearchHit {
                    id: s.id,
                    score: 0.0,
                    title: Some(s.topic.clone()),
                    snippet: s.summary.clone(),
                    kind: None,
                })
                .collect::<Vec<_>>()
        };
        let mut out_scenarios = Vec::new();
        for h in scenarios.into_iter().take(budget_items * 2 / 5) {
            match self.get_scenario(h.id).await {
                Ok(s)
                    if count(
                        &format!("{}{}", s.topic, s.summary),
                        &mut chars_used,
                        &mut truncated,
                    ) =>
                {
                    out_scenarios.push(s)
                }
                _ => break,
            }
        }

        // L1 补充（预算剩余）：有 query 按语义相关（search_atoms），否则 hit_count
        let remaining = budget_items.saturating_sub(persona.len() + out_scenarios.len());
        let atoms: Vec<AtomDto> = match query {
            Some(q) => {
                let hits =
                    search_atoms(&self.pool, q, qv.as_deref(), remaining as i64).await?;
                let ids: Vec<Uuid> = hits.iter().map(|h| h.id).collect();
                if ids.is_empty() {
                    vec![]
                } else {
                    let mut by_id: std::collections::HashMap<Uuid, AtomDto> =
                        sqlx::query_as::<_, AtomDto>("SELECT * FROM atoms WHERE id = ANY($1)")
                            .bind(&ids)
                            .fetch_all(&self.pool)
                            .await?
                            .into_iter()
                            .map(|a| (a.id, a))
                            .collect();
                    ids.into_iter().filter_map(|id| by_id.remove(&id)).collect()
                }
            }
            None => sqlx::query_as(
                "SELECT * FROM atoms WHERE status = 'active' ORDER BY hit_count DESC, confidence DESC, created_at DESC LIMIT $1",
            )
            .bind(remaining as i64)
            .fetch_all(&self.pool)
            .await?,
        };
        let mut out_atoms = Vec::new();
        for a in atoms {
            if count(&a.content, &mut chars_used, &mut truncated) {
                out_atoms.push(a);
            } else {
                truncated = true;
                break;
            }
        }

        // B9：context_pack 也是使用（AI 冷启动读路径），同样计热度
        self.fire_hit_feedback(
            "atoms",
            out_atoms.iter().map(|a| a.id).collect(),
        );
        self.fire_hit_feedback(
            "scenarios",
            out_scenarios.iter().map(|s| s.id).collect(),
        );

        Ok(ContextPack {
            persona,
            scenarios: out_scenarios,
            atoms: out_atoms,
            meta: ContextMeta {
                chars_used,
                truncated,
                query: query.map(String::from),
            },
        })
    }
}

/// source_refs 中擦除指定会话（标记 erased，保留结构）。
fn mark_erased(refs: serde_json::Value, erased_id: Uuid) -> serde_json::Value {
    match refs {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|mut item| {
                    if item.get("session_id").and_then(|v| v.as_str())
                        == Some(erased_id.to_string().as_str())
                    {
                        item["erased"] = serde_json::Value::Bool(true);
                    }
                    item
                })
                .collect(),
        ),
        other => other,
    }
}
