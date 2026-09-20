//! `provider` 的实现切片（架构治理 2026-09-21：自 provider.rs 纯搬移，零行为变化）。

use super::*;

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
pub(crate) enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// 从响应头解析 Retry-After，缺省 2s。
pub(crate) fn retry_after_secs(resp: &reqwest::Response) -> std::time::Duration {
    resp.headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(retry_after_from_str)
        .unwrap_or(std::time::Duration::from_secs(2))
}

/// OpenAI 兼容 HTTP provider（/v1/chat/completions + /v1/embeddings）。
///
/// base_url 规范化：trim → 去尾部 `/` → 去尾部 `/v1`（大小写不敏感）→ 再去尾部 `/`。
/// 用户填 `https://api.xx.com` 或 `https://api.xx.com/v1`（含尾随空格）均归一为同一根，
/// 路径拼接统一为 `{root}/v1/...`——消除两种填写约定的歧义（MCP 黑盒测试 D1 根因）。
pub(crate) fn normalize_base_url(base_url: &str) -> String {
    let mut b = base_url.trim().trim_end_matches('/').to_string();
    if b.len() >= 3 && b[b.len() - 3..].eq_ignore_ascii_case("/v1") {
        b.truncate(b.len() - 3);
    }
    b.trim_end_matches('/').to_string()
}

/// OpenAI 兼容响应片段（只取需要的字段）。
#[derive(serde::Deserialize)]
pub(crate) struct ChatApiResp {
    #[serde(default)]
    pub(crate) choices: Vec<ChatChoice>,
    #[serde(default)]
    pub(crate) usage: Option<ApiUsage>,
}

#[derive(serde::Deserialize)]
pub(crate) struct ChatChoice {
    #[serde(default)]
    pub(crate) message: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
pub(crate) struct ApiUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: i64,
    #[serde(default)]
    pub(crate) completion_tokens: i64,
    #[serde(default)]
    pub(crate) total_tokens: i64,
}

#[derive(serde::Deserialize)]
pub(crate) struct EmbedApiResp {
    #[serde(default)]
    pub(crate) data: Vec<EmbedItem>,
    #[serde(default)]
    pub(crate) usage: Option<ApiUsage>,
}

#[derive(serde::Deserialize)]
pub(crate) struct EmbedItem {
    pub(crate) embedding: Vec<f32>,
}

/// 把 reqwest 错误分类为瞬态/永久。
pub(crate) fn classify_http_error(status: Option<reqwest::StatusCode>) -> LlmError {
    match status {
        Some(s) if s.as_u16() == 429 || s.is_server_error() => {
            LlmError::Transient(format!("HTTP {s}"))
        }
        Some(s) => LlmError::Permanent(format!("HTTP {s}")),
        // 无状态码 = 网络/超时
        None => LlmError::Transient("网络错误或超时".into()),
    }
}

/// 拉取 OpenAI 兼容供应商的模型 ID 列表（GET {base}/models，标准端点）。
/// 供应商未实现 /models 时返回 Err(Permanent)——前端回退手动输入模型 ID。
pub async fn fetch_model_ids(
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, crate::types::LlmError> {
    // 粘贴的地址/密钥常带尾随空白或换行——header 值被污染会直接 401
    let base_url = normalize_base_url(base_url);
    let api_key = api_key.trim();
    use crate::types::LlmError;
    // base_url 已规范化为根形式，模型列表统一 {root}/v1/models
    let url = format!("{base_url}/v1/models");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| LlmError::Permanent(format!("HTTP 客户端构建失败: {e}")))?;
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| LlmError::Permanent(format!("连接失败: {e}")))?;
    let resp = resp;
    let status = resp.status();
    if !status.is_success() {
        // 按状态分类，避免误导（401 是 Key 错误，不是端点不支持）
        let hint = match status.as_u16() {
            401 | 403 => "认证失败——请检查 API Key 是否正确",
            404 => "供应商可能不支持 /models 列表——请手动输入模型 ID",
            _ => "供应商返回错误——请稍后重试或手动输入模型 ID",
        };
        return Err(LlmError::Permanent(format!(
            "HTTP {}——{}",
            status.as_u16(),
            hint
        )));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| LlmError::Permanent(format!("响应解析失败: {e}")))?;
    let ids: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(LlmError::Permanent(
            "供应商返回空模型列表——请手动输入模型 ID".into(),
        ));
    }
    let mut ids = ids;
    ids.sort();
    Ok(ids)
}
