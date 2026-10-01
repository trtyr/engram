//! 统一错误模型 + 错误码注册表（P006-T001）。
//!
//! 归因分类学（Q003 已决，目的=可调试性）：
//! - `ExternalInput`：用户/调用方输入错——打回即学会，不改代码能修；
//! - `Upstream`：上游服务（LLM/嵌入/web-reader）故障——可重试，等恢复；
//! - `Network`：网络/超时——可重试，退避；
//! - `Auth`：认证/权限——调用方凭据问题；
//! - `InternalBug`：代码 bug/状态不一致——**必须告警级日志**（T003 反馈层升级）。
//!
//! 规则：每个面向外部的错误都必须有注册表登记的 code；注册表唯一性由测试强制
//! （`error_codes_unique`）。域内错误（WikiError/JobError/…）经 `impl From` 桥接到
//! 此模型（T002 传播链 sweep 分批迁移）。

use serde::Serialize;

/// 错误归因类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// 用户/调用方输入错
    ExternalInput,
    /// 上游服务故障（LLM/嵌入/web-reader…）
    Upstream,
    /// 网络/超时
    Network,
    /// 认证/权限
    Auth,
    /// 代码 bug / 状态不一致（告警级）
    InternalBug,
}

impl ErrorCategory {
    /// 默认重试建议（归因规则：外部输入错与内部 bug 重试无意义）。
    /// 注意：这只是默认建议——注册表的 `retryable` 是显式声明，可覆盖默认
    /// （如 AUTH-RATE-LIMITED：锁定窗口过后可重试）。
    pub fn retryable(self) -> bool {
        matches!(self, ErrorCategory::Upstream | ErrorCategory::Network)
    }
}

/// 错误码定义（注册表条目）。
#[derive(Debug, Clone, Copy)]
pub struct ErrorCodeDef {
    /// 域前缀码（`域-语义`，如 `LLM-NOT-CONFIGURED`）——人读可猜、agent 可判断（Q002 已决）。
    pub code: &'static str,
    pub category: ErrorCategory,
    pub retryable: bool,
    pub description: &'static str,
}

const fn def(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    description: &'static str,
) -> ErrorCodeDef {
    ErrorCodeDef {
        code,
        category,
        retryable,
        description,
    }
}

/// 全系统错误码注册表——**新增错误码必须在此登记**（`error_codes_unique` 测试强制）。
/// 登记即文档：码/分类/可重试性/含义（T005 导出为《错误码全表》）。
pub const ERROR_CODES: &[ErrorCodeDef] = &[
    // ---- LLM 域 ----
    def(
        "LLM-NOT-CONFIGURED",
        ErrorCategory::ExternalInput,
        false,
        "未配置任何可用 provider——控制台先建 provider 并设默认（配置缺失是运维输入问题，重试无意义）",
    ),
    def(
        "LLM-CIRCUIT-OPEN",
        ErrorCategory::Upstream,
        true,
        "熔断器打开快速失败——等冷却恢复或修 provider 配置",
    ),
    def(
        "LLM-RATE-LIMITED",
        ErrorCategory::Upstream,
        true,
        "上游 429——退避重试",
    ),
    def(
        "LLM-AUTH-REJECTED",
        ErrorCategory::Auth,
        false,
        "上游 401/403——检查 API key",
    ),
    def(
        "LLM-BAD-RESPONSE",
        ErrorCategory::Upstream,
        true,
        "上游响应解析失败/维度不符——瞬态重试或换模型",
    ),
    // ---- wiki 文档域 ----
    def(
        "WIKI-DOC-NOT-FOUND",
        ErrorCategory::ExternalInput,
        false,
        "文档 id 不存在",
    ),
    def(
        "WIKI-DOC-URL-FETCH-FAILED",
        ErrorCategory::Network,
        true,
        "URL 抓取失败（429/5xx/网络）——已重试耗尽",
    ),
    def(
        "WIKI-DOC-SSRF-REJECTED",
        ErrorCategory::ExternalInput,
        false,
        "目标地址私网/保留段/协议不允许——安全策略拒绝",
    ),
    def(
        "WIKI-DOC-PARSE-EMPTY",
        ErrorCategory::InternalBug,
        false,
        "解析后内容为空——格式支持或解析器问题",
    ),
    def(
        "WIKI-DOC-EMBED-DEGRADED",
        ErrorCategory::Upstream,
        true,
        "嵌入失败降级 FTS——可事后 re-embed 补向量",
    ),
    // ---- 通用 ----
    def(
        "AUTH-FORBIDDEN",
        ErrorCategory::Auth,
        false,
        "scope/权限不足",
    ),
    def(
        "AUTH-INVALID-CREDENTIALS",
        ErrorCategory::Auth,
        false,
        "用户名/密码/key 无效",
    ),
    def(
        "AUTH-RATE-LIMITED",
        ErrorCategory::Auth,
        true,
        "登录防爆破锁定窗口内",
    ),
    def(
        "STORAGE-UNAVAILABLE",
        ErrorCategory::Network,
        true,
        "存储暂时不可用（连接/超时）",
    ),
    def(
        "INPUT-INVALID",
        ErrorCategory::ExternalInput,
        false,
        "参数校验失败——修正调用参数",
    ),
    def(
        "INPUT-TOO-LARGE",
        ErrorCategory::ExternalInput,
        false,
        "载荷超上限",
    ),
    def(
        "INTERNAL-INCONSISTENT",
        ErrorCategory::InternalBug,
        false,
        "内部状态不一致（孤儿态/约束违例）——必须告警",
    ),
];

/// 按 code 查注册表条目。
pub fn lookup(code: &str) -> Option<&'static ErrorCodeDef> {
    ERROR_CODES.iter().find(|d| d.code == code)
}

/// 统一错误：域错误的上层表达（边界输出格式 + 审计/日志的归因依据）。
#[derive(Debug, Clone, Serialize)]
pub struct EngramError {
    /// 注册表登记的码
    pub code: &'static str,
    pub category: ErrorCategory,
    /// 人读消息（可直接展示）
    pub message: String,
    /// 结构化上下文（资源类型/id/操作等——调试定位用）
    pub context: serde_json::Value,
    /// 根因链（底层错误的字符串表达——保留完整传播路径）
    pub source: Option<String>,
}

impl EngramError {
    /// 从注册表构造（code 必须已登记——未登记会 panic，编译期抓不到的用 `new_unregistered` 并尽快补登记）。
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        let def = lookup(code).unwrap_or_else(|| {
            panic!("错误码 {code} 未在 ERROR_CODES 注册——先登记再使用（P006-T001 纪律）")
        });
        Self {
            code,
            category: def.category,
            message: message.into(),
            context: serde_json::Value::Null,
            source: None,
        }
    }

    /// 附结构化上下文（链式）。
    pub fn with_context(mut self, key: &str, value: impl Serialize) -> Self {
        let mut ctx = match self.context {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        ctx.insert(key.to_string(), serde_json::json!(value));
        self.context = serde_json::Value::Object(ctx);
        self
    }

    /// 附根因链。
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// 归因分类的可重试性。
    pub fn retryable(&self) -> bool {
        self.category.retryable()
    }
}

impl std::fmt::Display for EngramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]", self.code)?;
        if !self.context.is_null() && self.context != serde_json::Value::Null {
            write!(f, " {}", self.context)?;
        }
        write!(f, " {}", self.message)?;
        if let Some(src) = &self.source {
            write!(f, "（根因: {src}）")?;
        }
        Ok(())
    }
}

impl std::error::Error for EngramError {}

/// WikiDocumentError → 统一错误桥（P006-T002 首批：EN-32 故障域示范）。
/// 归因映射：NotFound/BadRequest=调用方输入；Storage=网络/存储层。
impl From<&crate::wiki_docs::WikiDocumentError> for EngramError {
    fn from(e: &crate::wiki_docs::WikiDocumentError) -> Self {
        use crate::wiki_docs::WikiDocumentError;
        let (code, category) = match e {
            WikiDocumentError::NotFound(_) => ("WIKI-DOC-NOT-FOUND", ErrorCategory::ExternalInput),
            WikiDocumentError::BadRequest(_) => ("INPUT-INVALID", ErrorCategory::ExternalInput),
            WikiDocumentError::Storage(_) => ("STORAGE-UNAVAILABLE", ErrorCategory::Network),
        };
        Self {
            code,
            category,
            message: e.to_string(),
            context: serde_json::Value::Null,
            source: Some(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表唯一性 + 可重试性与分类一致（P006-T001 契约）。
    #[test]
    fn error_codes_unique_and_consistent() {
        let mut seen = std::collections::HashSet::new();
        for d in ERROR_CODES {
            assert!(seen.insert(d.code), "错误码重复: {}", d.code);
            assert!(!d.code.is_empty());
            assert!(
                d.code
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '-' || c.is_ascii_digit()),
                "码格式应为大写字母-连字符: {}",
                d.code
            );
            assert!(!d.description.is_empty(), "{} 缺描述", d.code);
            // 强不变式：内部 bug 重试无意义（其余类别允许显式覆盖默认建议）
            if d.category == ErrorCategory::InternalBug {
                assert!(!d.retryable, "{} InternalBug 不可重试", d.code);
            }
        }
    }

    #[test]
    fn engram_error_builds_with_context_and_source() {
        let e = EngramError::new("WIKI-DOC-NOT-FOUND", "文档不存在")
            .with_context("doc_id", "01a0")
            .with_source("SELECT 返回 0 行");
        assert_eq!(e.category, ErrorCategory::ExternalInput);
        assert!(!e.retryable());
        assert_eq!(e.context["doc_id"], "01a0");
        let display = e.to_string();
        assert!(display.contains("WIKI-DOC-NOT-FOUND"));
        assert!(display.contains("根因"));
    }

    /// From 桥：WikiDocumentError 归因映射（P006-T002 首批）。
    #[test]
    fn wiki_doc_error_bridge() {
        use crate::wiki_docs::WikiDocumentError;
        let e: EngramError = (&WikiDocumentError::NotFound("文档 x 不存在".into())).into();
        assert_eq!(e.code, "WIKI-DOC-NOT-FOUND");
        assert_eq!(e.category, ErrorCategory::ExternalInput);
        assert!(e.source.is_some());
        let e: EngramError = (&WikiDocumentError::Storage("连接超时".into())).into();
        assert_eq!(e.category, ErrorCategory::Network);
        assert!(e.retryable());
    }

    #[test]
    fn unregistered_code_panics() {
        let result = std::panic::catch_unwind(|| EngramError::new("NOT-REGISTERED", "x"));
        assert!(result.is_err(), "未注册码应 panic（登记纪律强制）");
    }
}
