//! Provider 抽象 + OpenAI 兼容实现 + 注册表（含用量记账）。

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{
    ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, LlmError, ModelInfo, UsageRecord,
};

/// Provider 能力 trait。mock 与真实实现共用；测试注入换实现即可。
pub trait LlmProvider: Send + Sync {
    /// 非流式 chat 补全。
    fn chat(
        &self,
        req: ChatRequest,
    ) -> impl std::future::Future<Output = Result<ChatResponse, LlmError>> + Send;
    /// 批量嵌入。
    fn embed(
        &self,
        req: EmbedRequest,
    ) -> impl std::future::Future<Output = Result<EmbedResponse, LlmError>> + Send;
    /// provider 名（记账用）。
    fn name(&self) -> &str;
}

/// OpenAI 兼容 HTTP provider（/v1/chat/completions + /v1/embeddings）。
pub struct OpenAiCompatProvider {
    name: String,
    base_url: String,
    api_key: String,
    http: reqwest::Client,
    /// chat 超时（默认 120s）
    chat_timeout: std::time::Duration,
    /// embed 超时（默认 30s）
    embed_timeout: std::time::Duration,
}

impl OpenAiCompatProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
            chat_timeout: std::time::Duration::from_secs(120),
            embed_timeout: std::time::Duration::from_secs(30),
        }
    }
}

/// OpenAI 兼容响应片段（只取需要的字段）。
#[derive(serde::Deserialize)]
struct ChatApiResp {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

#[derive(serde::Deserialize)]
struct EmbedApiResp {
    #[serde(default)]
    data: Vec<EmbedItem>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(serde::Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
}

/// 把 reqwest 错误分类为瞬态/永久。
fn classify_http_error(status: Option<reqwest::StatusCode>) -> LlmError {
    match status {
        Some(s) if s.as_u16() == 429 || s.is_server_error() => {
            LlmError::Transient(format!("HTTP {s}"))
        }
        Some(s) => LlmError::Permanent(format!("HTTP {s}")),
        // 无状态码 = 网络/超时
        None => LlmError::Transient("网络错误或超时".into()),
    }
}

impl LlmProvider for OpenAiCompatProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let started = std::time::Instant::now();
        let mut body = serde_json::json!({
            "model": req.model,
            "messages": req.messages,
        });
        if let Some(t) = req.temperature {
            body["temperature"] = serde_json::json!(t);
        }
        if let Some(m) = req.max_tokens {
            body["max_tokens"] = serde_json::json!(m);
        }
        if req.json_mode {
            body["response_format"] = serde_json::json!({ "type": "json_object" });
        }

        let resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(self.chat_timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| classify_http_error(e.status()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, body = %text.chars().take(500).collect::<String>(), "LLM chat 失败");
            return Err(classify_http_error(Some(status)));
        }

        let api: ChatApiResp = resp
            .json()
            .await
            .map_err(|e| LlmError::Permanent(format!("响应解析失败: {e}")))?;
        let content = api
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| LlmError::Permanent("响应缺少 content".into()))?
            .to_string();
        let usage = api.usage.unwrap_or(ApiUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        });

        Ok(ChatResponse {
            content,
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            model: req.model,
            latency_ms: started.elapsed().as_millis() as i64,
        })
    }

    async fn embed(&self, req: EmbedRequest) -> Result<EmbedResponse, LlmError> {
        let started = std::time::Instant::now();
        let mut body = serde_json::json!({ "model": req.model, "input": req.inputs });
        if let Some(d) = req.dimensions {
            body["dimensions"] = serde_json::json!(d);
        }

        let resp = self
            .http
            .post(format!("{}/v1/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(self.embed_timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| classify_http_error(e.status()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, body = %text.chars().take(500).collect::<String>(), "LLM embed 失败");
            return Err(classify_http_error(Some(status)));
        }

        let api: EmbedApiResp = resp
            .json()
            .await
            .map_err(|e| LlmError::Permanent(format!("响应解析失败: {e}")))?;
        let mut embeddings: Vec<(usize, Vec<f32>)> = Vec::new();
        for (i, item) in api.data.into_iter().enumerate() {
            embeddings.push((i, item.embedding));
        }
        // 按 index 排序后提取（多数服务已有序，防御性处理）
        embeddings.sort_by_key(|(i, _)| *i);
        let embeddings: Vec<Vec<f32>> = embeddings.into_iter().map(|(_, v)| v).collect();
        let usage = api.usage.unwrap_or(ApiUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        });

        Ok(EmbedResponse {
            embeddings,
            input_tokens: usage.total_tokens.max(usage.prompt_tokens),
            model: req.model,
            latency_ms: started.elapsed().as_millis() as i64,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Provider 注册表：读 DB 配置构建实例 + 记账。
#[derive(Clone)]
pub struct ProviderRegistry {
    pool: PgPool,
    cipher: crate::crypto::KeyCipher,
}

/// DB 行。
#[derive(Debug, sqlx::FromRow)]
struct ProviderRow {
    #[allow(dead_code, reason = "SELECT * 伴随字段")]
    id: Uuid,
    name: String,
    base_url: String,
    api_key_encrypted: Vec<u8>,
    #[allow(dead_code, reason = "SELECT * 伴随字段")]
    models: sqlx::types::Json<Vec<ModelInfo>>,
    #[allow(dead_code, reason = "SELECT * 伴随字段")]
    is_default: bool,
}

impl ProviderRegistry {
    pub fn new(pool: PgPool, cipher: crate::crypto::KeyCipher) -> Self {
        Self { pool, cipher }
    }

    /// 测试专用：共享内部池。
    #[doc(hidden)]
    pub fn pool_for_test(&self) -> PgPool {
        self.pool.clone()
    }

    /// 按名取 provider（每次从 DB 取，配置即时生效；单用户量级无性能问题）。
    pub async fn get(&self, name: &str) -> Result<Arc<OpenAiCompatProvider>, LlmError> {
        let row = sqlx::query_as::<_, ProviderRow>("SELECT * FROM llm_providers WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| LlmError::Transient(e.to_string()))?
            .ok_or_else(|| LlmError::NotConfigured(format!("provider 不存在: {name}")))?;

        let api_key = self.cipher.decrypt(&row.api_key_encrypted)?;
        Ok(Arc::new(OpenAiCompatProvider::new(
            row.name,
            row.base_url,
            api_key,
        )))
    }

    /// 默认 provider。
    pub async fn default_provider(&self) -> Result<Arc<OpenAiCompatProvider>, LlmError> {
        let row = sqlx::query_as::<_, ProviderRow>(
            "SELECT * FROM llm_providers WHERE is_default = true LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| LlmError::Transient(e.to_string()))?
        .ok_or_else(|| LlmError::NotConfigured("未配置默认 provider".into()))?;

        let api_key = self.cipher.decrypt(&row.api_key_encrypted)?;
        Ok(Arc::new(OpenAiCompatProvider::new(
            row.name,
            row.base_url,
            api_key,
        )))
    }

    /// 按用途解析 (provider, model)：路由链优先；回退默认 provider 中
    /// **具备对应能力**的模型（Embed→embedding 能力，chat 用途→非 embedding）。
    pub async fn resolve(
        &self,
        purpose: crate::types::Purpose,
    ) -> Result<(std::sync::Arc<OpenAiCompatProvider>, String), LlmError> {
        let table = crate::router::PurposeRouter::new(self.pool_for_test())
            .table()
            .await?;
        for rule in table.chain(purpose) {
            if let Ok(p) = self.get(&rule.provider).await {
                return Ok((p, rule.model.clone()));
            }
        }
        // 默认 provider + 按能力选模型
        type DefaultRow = (String, String, Vec<u8>, sqlx::types::Json<Vec<ModelInfo>>);
        let row: Option<DefaultRow> =
            sqlx::query_as(
                "SELECT name, base_url, api_key_encrypted, models FROM llm_providers WHERE is_default = true LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| LlmError::Transient(e.to_string()))?;
        let Some((name, base_url, enc, models)) = row else {
            return Err(LlmError::NotConfigured("未配置任何 LLM provider".into()));
        };
        let want_embed = purpose == crate::types::Purpose::Embed;
        let model = models
            .0
            .iter()
            .find(|m| {
                let has_emb = m.capabilities.iter().any(|c| c == "embedding");
                want_embed == has_emb
            })
            .or_else(|| models.0.first())
            .map(|m| m.id.clone())
            .ok_or_else(|| LlmError::NotConfigured(format!("provider {name} 未配置模型")))?;
        let api_key = self.cipher.decrypt(&enc)?;
        Ok((
            Arc::new(OpenAiCompatProvider::new(name, base_url, api_key)),
            model,
        ))
    }

    /// 记账（失败静默——记账故障不该影响业务，但要留日志）。
    pub async fn record_usage(&self, u: &crate::types::UsageMeta) {
        let res = sqlx::query(
            "INSERT INTO llm_usage (provider, model, purpose, input_tokens, output_tokens, latency_ms, job_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&u.provider)
        .bind(&u.model)
        .bind(&u.purpose)
        .bind(u.input_tokens)
        .bind(u.output_tokens)
        .bind(u.latency_ms as i32)
        .bind(u.job_id)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, "用量记账失败");
        }
    }

    /// 用量聚合查询（UI Dashboard 用）。
    pub async fn usage_summary(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<UsageRecord>, LlmError> {
        sqlx::query_as::<_, UsageRecord>(
            "SELECT * FROM llm_usage WHERE ts >= $1 ORDER BY ts DESC LIMIT 500",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| LlmError::Transient(e.to_string()))
    }
}
