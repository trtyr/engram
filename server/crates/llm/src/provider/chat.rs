//! `provider` 的实现切片（架构治理 2026-09-21：自 provider.rs 纯搬移，零行为变化）。

use super::*;
use crate::types::ToolCall;

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
            // http1_only：部分自建网关（如 newapi）HTTP/2 路径对大 body 不稳（5xx）——
            // HTTP/1.1 全兼容（P004-T010 demo 实测）
            http: reqwest::Client::builder()
                .http1_only()
                // 无 UA 会被部分网关前置层（WAF）拦成 5xx——显式带 UA（P004-T010 demo 实测）
                .user_agent(concat!("engram-llm/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
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
        if let Some(extras) = &req.extras {
            for (k, v) in extras {
                body[k.as_str()] = v.clone();
            }
        }
        // 回填消息里的 assistant.tool_calls 标准化：内部 ToolCall 是平铺 {id,name,arguments}，
        // OpenAI 线格式要求 {id,type:"function",function:{name,arguments}}——非标会被部分
        // 渠道上游按异常路径处理（P004-T010 demo 实证：非标+tool 回填稳定坏响应）
        if let Some(msgs) = body["messages"].as_array_mut() {
            for m in msgs.iter_mut() {
                if m["role"] == "assistant"
                    && let Some(calls) = m.get("tool_calls").and_then(|t| t.as_array()).cloned()
                {
                    let wrapped: Vec<serde_json::Value> = calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c["id"],
                                "type": "function",
                                "function": {
                                    "name": c["name"],
                                    "arguments": c["arguments"]
                                }
                            })
                        })
                        .collect();
                    m["tool_calls"] = serde_json::json!(wrapped);
                }
            }
        }
        // P004-T010：tool-calling（OpenAI function 格式）
        if let Some(tools) = &req.tools {
            body["tools"] = serde_json::json!(
                tools
                    .iter()
                    .map(|t| serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    }))
                    .collect::<Vec<_>>()
            );
        }

        // 诊断开关：ENGRAM_DUMP_REQ_PATH 设置时把请求体落盘（P004 demo 排查）
        if let Ok(p) = std::env::var("ENGRAM_DUMP_REQ_PATH") {
            let _ = std::fs::write(&p, body.to_string().as_bytes());
        }
        let resp = self
            .post_with_retry("/v1/chat/completions", self.chat_timeout, &body)
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            // P018-Q007：不再记录响应体内容（可能回显账号/配额信息），只记长度供排障
            tracing::warn!(status = %status, body_len = text.len(), "LLM chat 失败");
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
        // 工具调用轮 content 可能为 null——不再强制要求
        let first = api.choices.first().and_then(|c| c.message.as_ref());
        let content = first
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        // OpenAI tool_calls：[{"id","type":"function","function":{"name","arguments"}}]
        let tool_calls: Option<Vec<ToolCall>> = first
            .and_then(|m| m.get("tool_calls"))
            .and_then(|tc| tc.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| {
                        Some(ToolCall {
                            id: c["id"].as_str()?.to_string(),
                            name: c["function"]["name"].as_str()?.to_string(),
                            arguments: c["function"]["arguments"]
                                .as_str()
                                .unwrap_or("{}")
                                .to_string(),
                        })
                    })
                    .collect()
            });
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
            tool_calls,
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
