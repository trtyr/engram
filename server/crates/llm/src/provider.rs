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

/// 熔断器状态。
#[derive(Debug, Clone, Copy, PartialEq)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// 简单熔断器：连续失败达阈值 → Open（快速失败），冷却后 → HalfOpen（试探一次）。
#[derive(Debug)]
struct CircuitBreaker {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<std::time::Instant>,
    failure_threshold: u32,
    cooldown: std::time::Duration,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            failure_threshold: 5,
            cooldown: std::time::Duration::from_secs(30),
        }
    }
}

impl CircuitBreaker {
    /// 是否放行本次调用（Open 冷却期内快速失败；HalfOpen 只放行第一个试探请求）。
    fn allow(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if self
                    .opened_at
                    .map(|t| t.elapsed() >= self.cooldown)
                    .unwrap_or(true)
                {
                    self.state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
            // 试探请求未决期间：其余请求继续快速失败，避免半开瞬间放量冲击上游
            CircuitState::HalfOpen => false,
        }
    }

    fn record_success(&mut self) {
        self.state = CircuitState::Closed;
        self.consecutive_failures = 0;
        self.opened_at = None;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.state == CircuitState::HalfOpen
            || self.consecutive_failures >= self.failure_threshold
        {
            self.state = CircuitState::Open;
            self.opened_at = Some(std::time::Instant::now());
        }
    }
}

/// 全局熔断器注册表（按 provider name 共享，跨 resolve 实例有效）。
static CIRCUITS: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<CircuitBreaker>>>,
    >,
> = std::sync::LazyLock::new(|| {
    std::sync::Mutex::new(std::collections::HashMap::new())
});

/// 解析 Retry-After 头：支持 delta-seconds（整数秒）与 HTTP-date（RFC 2822）两种形式。
/// HTTP-date 已过期视为 0s（立即重试）；无法解析保守取 2s。
fn retry_after_from_str(raw: &str) -> std::time::Duration {
    let raw = raw.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return std::time::Duration::from_secs(secs);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(raw) {
        let delta = dt.timestamp() - chrono::Utc::now().timestamp();
        return if delta > 0 {
            std::time::Duration::from_secs(delta as u64)
        } else {
            std::time::Duration::ZERO
        };
    }
    std::time::Duration::from_secs(2)
}

/// 从响应头解析 Retry-After，缺省 2s。
fn retry_after_secs(resp: &reqwest::Response) -> std::time::Duration {
    resp.headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(retry_after_from_str)
        .unwrap_or(std::time::Duration::from_secs(2))
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
    /// 熔断器（按 name 全局共享）
    circuit: std::sync::Arc<std::sync::Mutex<CircuitBreaker>>,
}

impl OpenAiCompatProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let circuit = CIRCUITS
            .lock()
            .unwrap()
            .entry(name.clone())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(CircuitBreaker::default())))
            .clone();
        Self {
            name,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
            chat_timeout: std::time::Duration::from_secs(120),
            embed_timeout: std::time::Duration::from_secs(30),
            circuit,
        }
    }

    /// 发送 POST 并处理熔断 + 429 Retry-After 退避（最多重试 2 次）。
    async fn post_with_retry(
        &self,
        path: &str,
        timeout: std::time::Duration,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, LlmError> {
        if !self.circuit.lock().unwrap().allow() {
            return Err(LlmError::Transient("熔断器打开，快速失败".into()));
        }
        let url = format!("{}{}", self.base_url, path);
        let mut resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .timeout(timeout)
            .json(body)
            .send()
            .await
            .map_err(|e| {
                self.circuit.lock().unwrap().record_failure();
                classify_http_error(e.status())
            })?;
        for _ in 0..2 {
            if resp.status().as_u16() != 429 {
                break;
            }
            self.circuit.lock().unwrap().record_failure();
            tokio::time::sleep(retry_after_secs(&resp)).await;
            resp = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .timeout(timeout)
                .json(body)
                .send()
                .await
                .map_err(|e| {
                    self.circuit.lock().unwrap().record_failure();
                    classify_http_error(e.status())
                })?;
        }
        Ok(resp)
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
            .post_with_retry("/v1/chat/completions", self.chat_timeout, &body)
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, body = %text.chars().take(500).collect::<String>(), "LLM chat 失败");
            self.circuit.lock().unwrap().record_failure();
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

        self.circuit.lock().unwrap().record_success();
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
        let inputs = req.inputs.clone();
        let mut body = serde_json::json!({ "model": req.model, "input": inputs });
        if let Some(d) = req.dimensions {
            body["dimensions"] = serde_json::json!(d);
        }

        let mut resp = self
            .post_with_retry("/v1/embeddings", self.embed_timeout, &body)
            .await?;

        // 兼容：部分上游（如硅基流动 bge-m3）不支持 dimensions 参数，4xx 时去掉重试一次，
        // 靠模型默认维度（本项目统一 1024 维的模型默认即 1024）。
        if req.dimensions.is_some() && resp.status().is_client_error() {
            tracing::warn!(
                status = %resp.status(),
                model = %req.model,
                "上游拒绝 dimensions 参数，去掉后重试"
            );
            let body_no_dim = serde_json::json!({ "model": req.model, "input": req.inputs });
            resp = self
                .post_with_retry("/v1/embeddings", self.embed_timeout, &body_no_dim)
                .await?;
        }

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, body = %text.chars().take(500).collect::<String>(), "LLM embed 失败");
            self.circuit.lock().unwrap().record_failure();
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

        self.circuit.lock().unwrap().record_success();
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

    /// L6：嵌入记账门面——resolve + embed + record_usage 一体。调用方不再持有
    /// 裸 provider，记账从「约定」变「结构保证」（此前 knowledge 批量嵌入与
    /// 三域检索的查询嵌入全部绕过记账，用量面板系统性低估）。
    pub async fn embed_for(
        &self,
        purpose: crate::types::Purpose,
        inputs: Vec<String>,
        dimensions: Option<u32>,
        job_id: Option<Uuid>,
    ) -> Result<crate::types::EmbedResponse, LlmError> {
        let (provider, model) = self.resolve(purpose).await?;
        let provider_name = provider.name().to_string();
        let resp = provider
            .embed(crate::types::EmbedRequest {
                model,
                inputs,
                dimensions,
            })
            .await?;
        self.record_usage(&crate::types::UsageMeta {
            provider: provider_name,
            model: resp.model.clone(),
            purpose: purpose.as_str().to_string(),
            input_tokens: resp.input_tokens,
            output_tokens: 0,
            latency_ms: resp.latency_ms,
            job_id,
        })
        .await;
        Ok(resp)
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

    /// 默认 provider（L3：ORDER BY 保证多行残留时的确定性——正常路径唯一性由创建端维护）。
    pub async fn default_provider(&self) -> Result<Arc<OpenAiCompatProvider>, LlmError> {
        let row = sqlx::query_as::<_, ProviderRow>(
            "SELECT * FROM llm_providers WHERE is_default = true ORDER BY created_at LIMIT 1",
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
            match self.get(&rule.provider).await {
                Ok(p) => return Ok((p, rule.model.clone())),
                // L4：幽灵路由不再静默——warn 留痕（typo/改名导致的失效路由可发现）
                Err(e) => tracing::warn!(
                    purpose = purpose.as_str(),
                    provider = %rule.provider,
                    error = %e,
                    "路由链规则失效，跳过（回落下一条或默认 provider）"
                ),
            }
        }
        // 默认 provider + 按能力选模型
        // L3：ORDER BY 兜底确定性——存量多 default 行（本修复前数据/直插库）时
        // 取最早创建的，不再依赖物理顺序（热路径：resolve 是所有默认选择的唯一入口）
        type DefaultRow = (String, String, Vec<u8>, sqlx::types::Json<Vec<ModelInfo>>);
        let row: Option<DefaultRow> =
            sqlx::query_as(
                "SELECT name, base_url, api_key_encrypted, models FROM llm_providers WHERE is_default = true ORDER BY created_at LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| LlmError::Transient(e.to_string()))?;
        let Some((name, base_url, enc, models)) = row else {
            return Err(LlmError::NotConfigured("未配置任何 LLM provider".into()));
        };
        let want_embed = purpose == crate::types::Purpose::Embed;
        // L1：能力找不到直接报配置错误——旧实现 or_else(first) 会把 embedding-only
        // provider 的第一个嵌入模型选为 chat 模型（静默地雷：全部调用 400 却不指根因）
        let model = models
            .0
            .iter()
            .find(|m| {
                let has_emb = m.capabilities.iter().any(|c| c == "embedding");
                want_embed == has_emb
            })
            .map(|m| m.id.clone())
            .ok_or_else(|| {
                LlmError::NotConfigured(format!(
                    "provider {name} 无{}能力的模型，请检查 models 的 capabilities 配置",
                    if want_embed { "embedding" } else { "chat" }
                ))
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn breaker(threshold: u32, cooldown: std::time::Duration) -> CircuitBreaker {
        CircuitBreaker {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            failure_threshold: threshold,
            cooldown,
        }
    }

    #[test]
    fn circuit_opens_after_threshold_failures() {
        let mut cb = breaker(3, std::time::Duration::from_secs(30));
        cb.record_failure();
        cb.record_failure();
        assert!(cb.allow(), "未达阈值仍应放行");
        cb.record_failure();
        assert!(!cb.allow(), "达阈值后应熔断快速失败");
    }

    #[test]
    fn circuit_half_opens_after_cooldown_and_closes_on_success() {
        let mut cb = breaker(1, std::time::Duration::from_millis(1));
        cb.record_failure();
        assert!(!cb.allow(), "立即熔断");
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(cb.allow(), "冷却后放行试探（HalfOpen）");
        cb.record_success();
        assert_eq!(cb.state, CircuitState::Closed);
        assert!(cb.allow());
    }

    #[test]
    fn circuit_success_resets_failure_count() {
        let mut cb = breaker(5, std::time::Duration::from_secs(30));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.consecutive_failures, 0);
        assert_eq!(cb.state, CircuitState::Closed);
    }

    #[test]
    fn circuit_half_open_admits_only_one_probe() {
        let mut cb = breaker(1, std::time::Duration::from_millis(1));
        cb.record_failure(); // → Open
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(cb.allow(), "冷却后放行第一个试探请求");
        assert!(
            !cb.allow(),
            "试探未决期间其余请求应快速失败，不放行"
        );
        assert_eq!(cb.state, CircuitState::HalfOpen);
        cb.record_success(); // 试探成功 → Closed
        assert!(cb.allow(), "恢复 Closed 后正常放行");
    }

    #[test]
    fn retry_after_supports_seconds_and_http_date() {
        // delta-seconds
        assert_eq!(retry_after_from_str("5"), std::time::Duration::from_secs(5));
        assert_eq!(retry_after_from_str(" 5 "), std::time::Duration::from_secs(5));
        // HTTP-date（RFC 2822）：未来 → 正秒数（2100 距今数十年，必然远大于 1e6 秒）
        let future = retry_after_from_str("Fri, 01 Jan 2100 00:00:00 GMT");
        assert!(future.as_secs() > 1_000_000, "future: {future:?}");
        // HTTP-date：过去 → 0s（立即重试）；注意 RFC 2822 要求星期与日期一致（2000-01-01 是周六）
        let past = retry_after_from_str("Sat, 01 Jan 2000 00:00:00 GMT");
        assert_eq!(past, std::time::Duration::ZERO);
        // 无法解析 → 保守 2s
        assert_eq!(retry_after_from_str("soon"), std::time::Duration::from_secs(2));
        assert_eq!(retry_after_from_str(""), std::time::Duration::from_secs(2));
    }
}
