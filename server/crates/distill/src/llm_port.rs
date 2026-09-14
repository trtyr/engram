//! 蒸馏管道的 LLM 端口：trait 抽象（测试可注入 mock）+ 真网关实现。

use std::sync::Arc;

use engram_jobs::types::JobError;
use engram_llm::provider::LlmProvider as _;
use engram_llm::types::{ChatMessage, ChatRequest, EmbedRequest, LlmError, Purpose};
use engram_llm::{KeyCipher, ProviderRegistry};
use uuid::Uuid;

/// 蒸馏用 LLM 能力（chat JSON + embedding）。
/// chat_json 返回的 boxed future 形态。
pub type ChatJsonFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<serde_json::Value, JobError>> + Send + 'a>,
>;
/// embed 返回的 boxed future 形态。
pub type EmbedFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<Vec<f32>>, JobError>> + Send + 'a>,
>;

pub trait DistillLlm: Send + Sync {
    /// 一次结构化对话：system+user → JSON 值。内部处理 JSON 解析重试（一次）。
    fn chat_json<'a>(
        &'a self,
        purpose: Purpose,
        system: &'a str,
        user: &'a str,
        job_id: Uuid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, JobError>> + Send + 'a>,
    >;

    /// 批量嵌入（统一 1024 维，D0010）。
    fn embed<'a>(&'a self, texts: &'a [String], job_id: Uuid) -> EmbedFuture<'a>;
}

fn to_job_err(e: LlmError) -> JobError {
    match e {
        LlmError::Transient(m) => JobError::Retryable(m),
        LlmError::Permanent(m) => JobError::Permanent(m),
        LlmError::NotConfigured(m) => JobError::Permanent(m),
    }
}

/// 去 markdown 围栏后解析 JSON；失败时依次尝试容错提取：
/// ①截取首个 `{`/`[` 到末个 `}`/`]`（剥掉 LLM 在 JSON 前后附加的说明文字）；
/// ②修复对象/数组字面量中的尾逗号（`,}` `,]`，字符串字面量内不动）。
pub fn parse_json_lenient(text: &str) -> Result<serde_json::Value, String> {
    let trimmed = text.trim();
    let body = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest)
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest)
    } else {
        trimmed
    };
    let body = body.trim();
    if let Ok(v) = serde_json::from_str(body) {
        return Ok(v);
    }
    let extracted = extract_json_body(body);
    let repaired = repair_trailing_commas(extracted);
    serde_json::from_str(repaired.trim()).map_err(|e| format!("JSON 解析失败: {e}"))
}

/// 截取首个 `{`/`[` 到末个 `}`/`]` 的片段；找不到结构边界则原样返回。
fn extract_json_body(s: &str) -> &str {
    let start = s.find(['{', '[']);
    let end = s.rfind(['}', ']']);
    match (start, end) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => s,
    }
}

/// 删除对象/数组字面量中紧邻 `}`/`]` 的尾逗号；跳过字符串字面量（防误伤内容）。
fn repair_trailing_commas(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut in_str = false;
    let mut escaped = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                out.push(c);
                i += 1;
            }
            b',' => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'}' || b[j] == b']') {
                    i += 1; // 丢弃尾逗号
                } else {
                    out.push(c);
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// 统一入口：chat 一次 → 解析失败带追加指令重试一次（所有实现共用）。
/// 每次调用（含重试）的完整 I/O 记入 job_events，可归因可回放。
pub async fn chat_json_retrying(
    ctx: &engram_jobs::JobContext,
    llm: &dyn DistillLlm,
    purpose: Purpose,
    system: &str,
    user: &str,
    job_id: Uuid,
) -> Result<serde_json::Value, JobError> {
    let call = |tag: &str, user_text: &str| {
        serde_json::json!({
            "purpose": purpose.as_str(),
            "tag": tag,
            "system": system,
            "user": user_text,
        })
    };

    match llm.chat_json(purpose, system, user, job_id).await {
        Ok(v) => {
            ctx.emit(
                "LLM 调用",
                Some(serde_json::json!({
                    "purpose": purpose.as_str(),
                    "attempt": 1,
                    "input": call("first", user),
                    "output": v,
                })),
            )
            .await
            .ok();
            tracing::info!(purpose = purpose.as_str(), job = %job_id, attempt = 1, "LLM 调用成功");
            Ok(v)
        }
        Err(first) => {
            tracing::warn!(%first, %job_id, "LLM 输出解析失败，追加指令重试");
            let strict = format!(
                "{user}\n\n注意：你的上一个回答不合法。请只输出合法 JSON，不要任何其他文字。"
            );
            match llm.chat_json(purpose, system, &strict, job_id).await {
                Ok(v) => {
                    ctx.emit(
                        "LLM 调用（重试成功）",
                        Some(serde_json::json!({
                            "purpose": purpose.as_str(),
                            "attempt": 2,
                            "first_error": first.to_string(),
                            "input": call("retry", &strict),
                            "output": v,
                        })),
                    )
                    .await
                    .ok();
                    tracing::info!(purpose = purpose.as_str(), job = %job_id, attempt = 2, "LLM 重试成功");
                    Ok(v)
                }
                Err(second) => {
                    ctx.emit(
                        "LLM 调用失败（两次）",
                        Some(serde_json::json!({
                            "purpose": purpose.as_str(),
                            "first_error": first.to_string(),
                            "second_error": second.to_string(),
                            "input": call("retry", &strict),
                        })),
                    )
                    .await
                    .ok();
                    Err(second)
                }
            }
        }
    }
}

/// 真实实现：ProviderRegistry（路由 + 记账）。chat_json 单发；重试由 [`chat_json_retrying`] 统一处理。
/// embedding 维度（R11 配置化）：读 `AGENT_MEMORY_EMBEDDING_DIMENSIONS`（默认 1024 = 既有
/// 表列维度）。启动时 config 层已校验 ≠1024 会拒启——此处对非法值 warn 回落 1024（纵深防御，
/// 兜住测试/工具进程绕过 config 直用 GatewayLlm 的路径）。
pub fn embedding_dimensions() -> u32 {
    static DIM: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *DIM.get_or_init(
        || match std::env::var("AGENT_MEMORY_EMBEDDING_DIMENSIONS") {
            Ok(v) => match v.trim().parse::<u32>() {
                Ok(d) if d > 0 => d,
                _ => {
                    tracing::warn!("AGENT_MEMORY_EMBEDDING_DIMENSIONS 非法（{v}）——回落 1024");
                    1024
                }
            },
            Err(_) => 1024,
        },
    )
}

pub struct GatewayLlm {
    registry: ProviderRegistry,
    /// 单 job token 预算（熔断）
    pub budget_tokens: i64,
}

impl GatewayLlm {
    pub fn new(pool: sqlx::PgPool, cipher: KeyCipher) -> Self {
        Self {
            registry: ProviderRegistry::new(pool, cipher),
            budget_tokens: 400_000,
        }
    }

    async fn chat_once(
        &self,
        purpose: Purpose,
        system: &str,
        user: &str,
        job_id: Uuid,
        extra_instruction: bool,
    ) -> Result<(String, i64), JobError> {
        let (provider, model) = self.registry.resolve(purpose).await.map_err(to_job_err)?;
        let mut messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        if extra_instruction {
            messages.push(ChatMessage::system(
                "注意：你的上一个回答不是合法 JSON。请只输出合法 JSON，不要任何其他文字。",
            ));
        }
        let resp = provider
            .chat(ChatRequest {
                model: model.clone(),
                messages,
                temperature: Some(0.1),
                json_mode: true,
                max_tokens: None,
            })
            .await
            .map_err(to_job_err)?;
        let total = resp.input_tokens + resp.output_tokens;
        self.registry
            .record_usage(&engram_llm::types::UsageMeta {
                provider: provider.name().to_string(),
                model,
                purpose: purpose.as_str().to_string(),
                input_tokens: resp.input_tokens,
                output_tokens: resp.output_tokens,
                latency_ms: resp.latency_ms,
                job_id: Some(job_id),
            })
            .await;
        Ok((resp.content, total))
    }
}

impl DistillLlm for GatewayLlm {
    fn chat_json<'a>(
        &'a self,
        purpose: Purpose,
        system: &'a str,
        user: &'a str,
        job_id: Uuid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, JobError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let (content, _used) = self.chat_once(purpose, system, user, job_id, false).await?;
            // LLM 输出非法 JSON 是暂时性错误（输出有随机性，任务级重试常能自愈）——
            // 分类为 Retryable 让队列的 max_attempts 生效；Permanent 会放弃剩余重试直接 failed。
            parse_json_lenient(&content).map_err(|e| {
                JobError::Retryable(format!("LLM 输出非法 JSON（已重试一次仍失败）: {e}"))
            })
        })
    }

    fn embed<'a>(
        &'a self,
        texts: &'a [String],
        job_id: Uuid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<Vec<f32>>, JobError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let (provider, model) = self
                .registry
                .resolve(Purpose::Embed)
                .await
                .map_err(to_job_err)?;
            let resp = provider
                .embed(EmbedRequest {
                    model: model.clone(),
                    inputs: texts.to_vec(),
                    dimensions: Some(embedding_dimensions()),
                })
                .await
                .map_err(to_job_err)?;
            self.registry
                .record_usage(&engram_llm::types::UsageMeta {
                    provider: provider.name().to_string(),
                    model,
                    purpose: Purpose::Embed.as_str().to_string(),
                    input_tokens: resp.input_tokens,
                    output_tokens: 0,
                    latency_ms: resp.latency_ms,
                    job_id: Some(job_id),
                })
                .await;
            Ok(resp.embeddings)
        })
    }
}

/// mock：按调用顺序弹出预置响应（FIFO，测试注入）。
pub struct MockLlm {
    /// chat 响应队列（可能是非法 JSON，用于测解析重试）
    pub chats: std::sync::Mutex<std::collections::VecDeque<String>>,
    /// embedding 固定输出维度
    pub embed_dim: usize,
    /// 测试注入（W3）：embed 一律失败
    pub embed_fail: bool,
    /// 测试注入（W3/K4）：embed 短响应（比输入少一条）
    pub embed_short: bool,
    /// 测试录制：chat_json 收到的 user prompt 逐条入列——断言「LLM 到底看见了什么」
    /// （P1：验证仲裁相似列表是否把无嵌入种子原子喂给了模型）
    pub sent_user: std::sync::Mutex<Vec<String>>,
}

impl MockLlm {
    pub fn with_chats(chats: Vec<serde_json::Value>) -> Self {
        Self::with_raw_chats(chats.into_iter().map(|v| v.to_string()).collect())
    }

    /// 原始响应文本（可含非法 JSON，测解析重试）。
    pub fn with_raw_chats(chats: Vec<String>) -> Self {
        Self {
            chats: std::sync::Mutex::new(chats.into_iter().collect()),
            embed_dim: 1024, // 与存储层 vector(1024) 一致（D0010）
            embed_fail: false,
            embed_short: false,
            sent_user: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl DistillLlm for MockLlm {
    fn chat_json<'a>(
        &'a self,
        _purpose: Purpose,
        _system: &'a str,
        _user: &'a str,
        _job_id: Uuid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, JobError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.sent_user.lock().unwrap().push(_user.to_string());
            let mut q = self.chats.lock().unwrap();
            match q.pop_front() {
                Some(s) => parse_json_lenient(&s)
                    .map_err(|e| JobError::Retryable(format!("LLM 输出非法 JSON: {e}"))),
                None => Err(JobError::Permanent("MockLlm 响应耗尽".into())),
            }
        })
    }

    fn embed<'a>(
        &'a self,
        texts: &'a [String],
        _job_id: Uuid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<Vec<f32>>, JobError>> + Send + 'a>,
    > {
        Box::pin(async move {
            // W3/K4 测试注入：失败 / 短响应路径
            if self.embed_fail {
                return Err(JobError::Permanent("mock embed 失败（注入）".into()));
            }
            let n = texts.len() - usize::from(self.embed_short && texts.len() > 1);
            Ok(texts
                .iter()
                .take(n)
                .map(|t| {
                    // 内容确定性的伪向量（相似文本相近：hash 混合）。
                    // +1 保证非零：B2 的零向量守卫会把全零嵌入过滤为 NULL
                    // （h % 97 == 0 的输入会碰撞出全零，实测「用户用 Mac 开发」命中）
                    let h = t.chars().map(|c| c as usize).sum::<usize>();
                    (0..self.embed_dim)
                        .map(|i| ((h.wrapping_mul(i + 7)) % 97 + 1) as f32 / 98.0)
                        .collect::<Vec<f32>>()
                })
                .collect())
        })
    }
}

pub type LlmRef = Arc<dyn DistillLlm>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_plain_json() {
        let v = parse_json_lenient(r#"{"a": 1}"#).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn lenient_code_fence() {
        let v = parse_json_lenient("```json\n{\"a\": [1, 2]}\n```").unwrap();
        assert_eq!(v["a"][1], 2);
    }

    #[test]
    fn lenient_prose_around_json() {
        // LLM 在 JSON 前后加说明文字——截取结构主体
        let raw = "好的，以下是分析结果：\n{\"topic\": \"场景\", \"score\": 0.9}\n希望有帮助！";
        let v = parse_json_lenient(raw).unwrap();
        assert_eq!(v["topic"], "场景");
    }

    #[test]
    fn lenient_trailing_commas() {
        let v = parse_json_lenient("{\"a\": 1, \"list\": [1, 2,],}").unwrap();
        assert_eq!(v["list"][1], 2);
        // 字符串字面量里的 ,} 不被误伤
        let v2 = parse_json_lenient("{\"text\": \"值A,}值B\", \"n\": 1}").unwrap();
        assert_eq!(v2["text"], "值A,}值B");
    }

    #[test]
    fn lenient_prose_plus_trailing_comma() {
        let raw = "结果如下：{\"items\": [{\"id\": \"x\",},],}\n以上。";
        let v = parse_json_lenient(raw).unwrap();
        assert_eq!(v["items"][0]["id"], "x");
    }

    #[test]
    fn lenient_garbage_still_rejected() {
        let err = parse_json_lenient("这不是 JSON，真的不是").unwrap_err();
        assert!(err.contains("JSON 解析失败"));
    }

    #[tokio::test]
    async fn mock_chat_json_parse_failure_is_retryable() {
        // 解析失败必须分类为 Retryable——Permanent 会让队列放弃 max_attempts 剩余重试
        let mock = MockLlm::with_raw_chats(vec!["完全不是 JSON".into()]);
        let r = mock.chat_json(
            engram_llm::types::Purpose::Organize,
            "sys",
            "user",
            uuid::Uuid::nil(),
        );
        let err = r.await.unwrap_err();
        assert!(
            matches!(err, JobError::Retryable(_)),
            "应为 Retryable，实际 {err:?}"
        );
    }
}
