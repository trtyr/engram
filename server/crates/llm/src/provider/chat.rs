//! `provider` 的实现切片（架构治理 2026-09-21：自 provider.rs 纯搬移，零行为变化）。

use super::*;

impl OpenAiCompatProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let circuit = CIRCUITS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(name.clone())
            .or_insert_with(|| {
                std::sync::Arc::new(std::sync::Mutex::new(CircuitBreaker::default()))
            })
            .clone();
        Self {
            name,
            base_url: normalize_base_url(base_url.into().as_str()),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
            chat_timeout: std::time::Duration::from_secs(120),
            embed_timeout: std::time::Duration::from_secs(30),
            circuit,
        }
    }

    /// 发送 POST 并处理熔断 + 429 Retry-After 退避（最多重试 2 次）。
    pub(crate) async fn post_with_retry(
        &self,
        path: &str,
        timeout: std::time::Duration,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, LlmError> {
        if !self
            .circuit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allow()
        {
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
                self.circuit
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_failure();
                classify_http_error(e.status())
            })?;
        for _ in 0..2 {
            if resp.status().as_u16() != 429 {
                break;
            }
            self.circuit
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record_failure();
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
                    self.circuit
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .record_failure();
                    classify_http_error(e.status())
                })?;
        }
        Ok(resp)
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
            self.circuit
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record_failure();
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

        self.circuit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_success();
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
            self.circuit
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record_failure();
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

        self.circuit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_success();
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
