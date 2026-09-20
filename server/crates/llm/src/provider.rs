//! Provider 抽象 + OpenAI 兼容实现 + 注册表（含用量记账）。

mod chat;
mod wire;
pub use wire::*;

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, LlmError, UsageRecord};

/// 简单熔断器：连续失败达阈值 → Open（快速失败），冷却后 → HalfOpen（试探一次）。
#[derive(Debug)]
pub(crate) struct CircuitBreaker {
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
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

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

/// Provider 注册表：读 DB 配置构建实例 + 记账。
#[derive(Clone)]
pub struct ProviderRegistry {
    pool: PgPool,
    cipher: crate::crypto::KeyCipher,
}

/// DB 行。
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct ProviderRow {
    #[allow(dead_code, reason = "SELECT * 伴随字段")]
    id: Uuid,
    name: String,
    base_url: String,
    api_key_encrypted: Vec<u8>,
    model_id: String,
    #[allow(dead_code, reason = "SELECT * 伴随字段")]
    capability: String,
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
    /// 裸 provider，记账从「约定」变「结构保证」（此前 wiki 文档批量嵌入与
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
    /// 返回 (provider, model_id)——一个 provider 一个模型。
    pub async fn get(&self, name: &str) -> Result<(Arc<OpenAiCompatProvider>, String), LlmError> {
        let row = sqlx::query_as::<_, ProviderRow>("SELECT * FROM llm_providers WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| LlmError::Transient(e.to_string()))?
            .ok_or_else(|| LlmError::NotConfigured(format!("provider 不存在: {name}")))?;

        let api_key = self.cipher.decrypt(&row.api_key_encrypted)?;
        Ok((
            Arc::new(OpenAiCompatProvider::new(row.name, row.base_url, api_key)),
            row.model_id,
        ))
    }

    /// 按用途解析 (provider, model)：路由链优先；回退默认 provider 中
    /// **对应能力**的一个（Embed→embedding，chat 用途→chat；一个 provider 一个模型）。
    pub async fn resolve(
        &self,
        purpose: crate::types::Purpose,
    ) -> Result<(std::sync::Arc<OpenAiCompatProvider>, String), LlmError> {
        let table = crate::router::PurposeRouter::new(self.pool_for_test())
            .table()
            .await?;
        for rule in table.chain(purpose) {
            match self.get(&rule.provider).await {
                Ok((p, model)) => return Ok((p, model)),
                // L4：幽灵路由不再静默——warn 留痕（typo/改名导致的失效路由可发现）
                Err(e) => tracing::warn!(
                    purpose = purpose.as_str(),
                    provider = %rule.provider,
                    error = %e,
                    "路由链规则失效，跳过（回落下一条或默认 provider）"
                ),
            }
        }
        // 默认回退：按能力选默认 provider（chat 与 embedding 各有一个默认）。
        // L3：ORDER BY 兜底确定性——存量多 default 行时取最早创建的。
        let want_embed = purpose == crate::types::Purpose::Embed;
        let capability = if want_embed { "embedding" } else { "chat" };
        let row: Option<(String, String, Vec<u8>, String)> = sqlx::query_as(
            "SELECT name, base_url, api_key_encrypted, model_id FROM llm_providers \
             WHERE is_default = true AND capability = $1 ORDER BY created_at LIMIT 1",
        )
        .bind(capability)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| LlmError::Transient(e.to_string()))?;
        let Some((name, base_url, enc, model_id)) = row else {
            return Err(LlmError::NotConfigured(format!(
                "未配置任何 {capability} LLM provider"
            )));
        };
        let api_key = self.cipher.decrypt(&enc)?;
        Ok((
            Arc::new(OpenAiCompatProvider::new(name, base_url, api_key)),
            model_id,
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
        assert!(!cb.allow(), "试探未决期间其余请求应快速失败，不放行");
        assert_eq!(cb.state, CircuitState::HalfOpen);
        cb.record_success(); // 试探成功 → Closed
        assert!(cb.allow(), "恢复 Closed 后正常放行");
    }

    #[test]
    fn retry_after_supports_seconds_and_http_date() {
        // delta-seconds
        assert_eq!(retry_after_from_str("5"), std::time::Duration::from_secs(5));
        assert_eq!(
            retry_after_from_str(" 5 "),
            std::time::Duration::from_secs(5)
        );
        // HTTP-date（RFC 2822）：未来 → 正秒数（2100 距今数十年，必然远大于 1e6 秒）
        let future = retry_after_from_str("Fri, 01 Jan 2100 00:00:00 GMT");
        assert!(future.as_secs() > 1_000_000, "future: {future:?}");
        // HTTP-date：过去 → 0s（立即重试）；注意 RFC 2822 要求星期与日期一致（2000-01-01 是周六）
        let past = retry_after_from_str("Sat, 01 Jan 2000 00:00:00 GMT");
        assert_eq!(past, std::time::Duration::ZERO);
        // 无法解析 → 保守 2s
        assert_eq!(
            retry_after_from_str("soon"),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(retry_after_from_str(""), std::time::Duration::from_secs(2));
    }
}
