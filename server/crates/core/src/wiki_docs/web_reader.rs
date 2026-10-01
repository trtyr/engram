//! 智谱 web-reader MCP 客户端（P004-T005，Q002 已决）。
//!
//! 端点：`https://open.bigmodel.cn/api/mcp/web_reader/mcp`（streamable HTTP，SSE 响应），
//! 工具 `webReader` 返回结构化 JSON（title/description/url/content 纯净 markdown + 链接表），
//! 对 JS 渲染页同样有效——质量显著优于本地裸 HTML 处理（2026-10-01 实测对比）。
//!
//! 配置：credentials 表 `zhipu/web_reader_key`（存在即启用）；base_url 可经
//! settings 键 `webreader_base_url` 覆盖（测试注入不可达端点用）。凭据纪律：
//! 明文只在解密后内存中流转，禁止落日志/文档。
//!
//! 失败语义：调用方（fetch_document_bytes）收到 Err 后回落本地 safe_fetch 并 emit 降级事件。

use engram_storage::PgPool;

/// 缺省端点（智谱官方 remote MCP）。
pub const DEFAULT_BASE_URL: &str = "https://open.bigmodel.cn/api/mcp/web_reader/mcp";
/// credentials 表约定名（T006 前端设置页写同名条目）。
pub const CREDENTIAL_NAME: &str = "zhipu/web_reader_key";

#[derive(Debug, thiserror::Error)]
pub enum WebReaderError {
    #[error("web-reader 响应异常: {0}")]
    BadResponse(String),
    #[error("web-reader 网络错误: {0}")]
    Network(String),
}

pub struct WebReaderClient {
    base: String,
    api_key: String,
    http: reqwest::Client,
}

/// 抓取产物：标题 + 正文 markdown。
pub struct WebReaderPage {
    pub title: Option<String>,
    pub content_markdown: String,
}

impl WebReaderClient {
    /// 从 pool 装配客户端（credentials 无 key → None = 未启用，走本地回落）。
    pub async fn from_pool(pool: &PgPool) -> Option<Self> {
        let hex = std::env::var("AGENT_MEMORY_MASTER_KEY").unwrap_or_else(|_| "00".repeat(32));
        let cipher = engram_llm::KeyCipher::from_hex_master(&hex).ok()?;
        let svc = crate::credentials::CredentialsService::new(pool.clone(), cipher);
        let cred = svc
            .get(CREDENTIAL_NAME, "wiki-docs-web-reader")
            .await
            .ok()?;
        let base = settings_get(pool, "webreader_base_url")
            .await
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Some(Self {
            base,
            api_key: cred.value,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .ok()?,
        })
    }

    /// 抓取一个 URL：initialize → initialized → tools/call webReader，返回 markdown 正文。
    pub async fn read_url(&self, url: &str) -> Result<WebReaderPage, WebReaderError> {
        // 1. initialize（取 session id）
        let resp = self
            .http
            .post(&self.base)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "clientInfo": {"name": "engram-wiki-docs", "version": "0.1"}
                    }
                })
                .to_string(),
            )
            .send()
            .await
            .map_err(|e| WebReaderError::Network(e.to_string()))?;
        let session = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| WebReaderError::BadResponse("缺 mcp-session-id".into()))?;
        // initialize 的 SSE body 丢弃（只要 session id）
        let _ = resp.text().await;

        // 2. initialized 通知
        let _ = self
            .http
            .post(&self.base)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("mcp-session-id", &session)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(
                serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
                    .to_string(),
            )
            .send()
            .await;

        // 3. tools/call webReader
        let resp = self
            .http
            .post(&self.base)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("mcp-session-id", &session)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": "webReader", "arguments": {"url": url}}
                })
                .to_string(),
            )
            .send()
            .await
            .map_err(|e| WebReaderError::Network(e.to_string()))?;
        let body = resp
            .text()
            .await
            .map_err(|e| WebReaderError::Network(e.to_string()))?;
        let payload = sse_data_payload(&body)
            .ok_or_else(|| WebReaderError::BadResponse("SSE 无 data 块".into()))?;
        let rpc: serde_json::Value = serde_json::from_str(payload)
            .map_err(|e| WebReaderError::BadResponse(format!("JSON-RPC 解析失败: {e}")))?;
        if let Some(err) = rpc.get("error") {
            return Err(WebReaderError::BadResponse(format!("远端错误: {err}")));
        }
        // result.content[0].text 是「字符串化的 JSON」（title/description/url/content）
        let text = rpc["result"]["content"][0]["text"]
            .as_str()
            .ok_or_else(|| WebReaderError::BadResponse("content[0].text 缺失".into()))?;
        let mut page: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| WebReaderError::BadResponse(format!("内嵌 JSON 解析失败: {e}")))?;
        // 智谱侧双重序列化：text 是「JSON 字符串再包一层引号」——解到 object 为止
        if let Some(inner) = page
            .as_str()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        {
            page = inner;
        }
        let content = page["content"]
            .as_str()
            .ok_or_else(|| WebReaderError::BadResponse("content 缺失".into()))?
            .to_string();
        if content.trim().is_empty() {
            return Err(WebReaderError::BadResponse("正文为空".into()));
        }
        Ok(WebReaderPage {
            title: page["title"].as_str().map(|s| s.to_string()),
            content_markdown: content,
        })
    }
}

/// SSE body 里取 JSON-RPC 结果块（正序第一个含 "jsonrpc" 的 data: 行——
/// 服务端可能附带其他事件块，倒序取尾会命中非结果块）。
fn sse_data_payload(body: &str) -> Option<&str> {
    body.lines().find_map(|l| {
        l.strip_prefix("data:")
            .map(|s| s.trim())
            .filter(|s| s.contains("jsonrpc") && !s.is_empty())
    })
}

/// settings 表单键读取（core src 层零 SQL——走 storage repo；缺键 → None）。
async fn settings_get(pool: &engram_storage::PgPool, key: &str) -> Option<String> {
    engram_storage::repo::settings::get_json::<String>(pool, key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_payload_takes_first_jsonrpc_block() {
        let body = "id:1\nevent:message\ndata:{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n\n";
        assert_eq!(
            sse_data_payload(body),
            Some("{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}")
        );
        // 多块：正序第一个 jsonrpc 块（尾部可能有非结果事件）
        let multi = "event:ping\ndata:keepalive\n\nid:1\ndata:{\"jsonrpc\":\"ok\"}\n\n";
        assert_eq!(sse_data_payload(multi), Some("{\"jsonrpc\":\"ok\"}"));
        assert_eq!(sse_data_payload("no data here"), None);
    }
}
