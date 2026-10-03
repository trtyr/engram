//! JEV 决策模型客户端（OpenRouter Decisions API，`typesafe/jev-1.13`）。
//!
//! 决策 001（2026-10-03）：System One 决策原语（choice/noul/score）作为蒸馏链与
//! KV 准入的「廉价哨兵」——typed 输出 + 概率，输出 token 免费。
//! 接口仅支持 OpenRouter（用户拍板：无 provider 选择器，默认且唯一）。
//!
//! 配置存 settings 单行 JSON（key=`jev`，同 mcp/rhythm 先例；读写壳在 api 层）；
//! api_key 经 KeyCipher 加密（hex 存储）。注意：换主密钥的 re-encrypt 链只扫
//! llm_providers——JEV key 需在设置页重新保存（此处明示，接受该限制）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::crypto::KeyCipher;
use crate::types::LlmError;

pub const SETTINGS_KEY: &str = "jev";
pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";
pub const DECISIONS_ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
pub const DEFAULT_REJECT_THRESHOLD: f64 = 0.3;
pub const DEFAULT_REVIEW_THRESHOLD: f64 = 0.5;

fn default_model() -> String {
    DEFAULT_MODEL.into()
}

fn default_reject() -> f64 {
    DEFAULT_REJECT_THRESHOLD
}

fn default_review() -> f64 {
    DEFAULT_REVIEW_THRESHOLD
}

/// JEV 配置（settings key=`jev`）。`api_key_enc` = KeyCipher 加密后 hex。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key_enc: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_reject")]
    pub reject_threshold: f64,
    #[serde(default = "default_review")]
    pub review_threshold: f64,
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_enc: None,
            model: default_model(),
            reject_threshold: default_reject(),
            review_threshold: default_review(),
        }
    }
}

impl JevConfig {
    pub fn key_configured(&self) -> bool {
        self.api_key_enc.as_ref().is_some_and(|s| !s.is_empty())
    }
}

/// GET 面的脱敏视图——key 永不回显，只给「已配置」布尔。
pub fn masked(cfg: &JevConfig) -> serde_json::Value {
    serde_json::json!({
        "enabled": cfg.enabled,
        "model": cfg.model,
        "reject_threshold": cfg.reject_threshold,
        "review_threshold": cfg.review_threshold,
        "key_configured": cfg.key_configured(),
        "provider": "openrouter",
    })
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 加密明文 key → hex（api 壳 PUT 时用）。
pub fn encrypt_key(plaintext: &str, cipher: &KeyCipher) -> Result<String, LlmError> {
    Ok(hex_encode(&cipher.encrypt(plaintext)?))
}

/// 解析出可用客户端。`Ok(None)` = 未启用或 key 缺失——调用方降级直通（决策 001：
/// 哨兵打盹不阻塞主流程）。
pub fn resolve(
    cfg: &JevConfig,
    cipher: &KeyCipher,
    http: reqwest::Client,
) -> Result<Option<JevClient>, LlmError> {
    if !cfg.enabled || !cfg.key_configured() {
        return Ok(None);
    }
    let enc = hex_decode(cfg.api_key_enc.as_deref().unwrap_or_default()).ok_or_else(|| {
        LlmError::Permanent("jev api_key_enc 不是合法 hex——重新保存一次 API key".into())
    })?;
    let api_key = cipher.decrypt(&enc)?;
    Ok(Some(JevClient {
        http,
        api_key,
        model: cfg.model.clone(),
        reject_threshold: cfg.reject_threshold,
        review_threshold: cfg.review_threshold,
    }))
}

/// 问题原语（cookbook 形态）。
pub enum Question {
    /// 互斥单选：criteria = 选项 → 判据描述。
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    /// 条件判定：返回 P(yes)。
    Noul { instructions: String },
}

impl Question {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Question::Choice {
                instructions,
                criteria,
            } => serde_json::json!({
                "type": "choice",
                "instructions": instructions,
                "criteria": criteria,
            }),
            Question::Noul { instructions } => serde_json::json!({
                "type": "noul",
                "instructions": instructions,
            }),
        }
    }
}

/// 单个问题的 typed 答案。
#[derive(Debug, Clone)]
pub enum Answer {
    Choice {
        choice: String,
        confidence: Option<f64>,
        probabilities: BTreeMap<String, f64>,
    },
    Noul(f64),
}

impl Answer {
    /// noul 概率（choice 答案返回 None）。
    pub fn noul(&self) -> Option<f64> {
        match self {
            Answer::Noul(p) => Some(*p),
            Answer::Choice { .. } => None,
        }
    }

    /// choice 选中项（noul 答案返回 None）。
    pub fn choice(&self) -> Option<&str> {
        match self {
            Answer::Choice { choice, .. } => Some(choice),
            Answer::Noul(_) => None,
        }
    }
}

/// 一次 Decisions 调用的全部答案 + 用量（cost 单位 USD）。
#[derive(Debug, Clone)]
pub struct DecisionsResult {
    pub answers: BTreeMap<String, Answer>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
}

/// JEV 决策客户端（resolve 构造；clone 安全）。
#[derive(Clone)]
pub struct JevClient {
    http: reqwest::Client,
    api_key: String,
    pub model: String,
    pub reject_threshold: f64,
    pub review_threshold: f64,
}

impl std::fmt::Debug for JevClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JevClient(<redacted>)")
    }
}

fn parse_answer(kind: &str, v: &serde_json::Value) -> Option<Answer> {
    match kind {
        "choice" => {
            let choice = v.get("choice")?.as_str()?.to_string();
            let confidence = v.get("confidence").and_then(|c| c.as_f64());
            let mut probabilities = BTreeMap::new();
            // alpha 响应的逐选项概率字段名出现过 probabilities/probability 两种——宽松双取
            for field in ["probabilities", "probability"] {
                if let Some(map) = v.get(field).and_then(|m| m.as_object()) {
                    for (k, p) in map {
                        if let Some(p) = p.as_f64() {
                            probabilities.insert(k.clone(), p);
                        }
                    }
                    if !probabilities.is_empty() {
                        break;
                    }
                }
            }
            Some(Answer::Choice {
                choice,
                confidence,
                probabilities,
            })
        }
        "noul" => Some(Answer::Noul(v.get("noul")?.as_f64()?)),
        _ => None,
    }
}

/// HTTP 状态分类（对齐 LlmError 语义：429/5xx/402 预算瞬时 = 可重试）。
fn classify_status(status: reqwest::StatusCode) -> LlmError {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::PAYMENT_REQUIRED
        || status.is_server_error()
    {
        LlmError::Transient(format!("JEV decisions HTTP {status}"))
    } else {
        LlmError::Permanent(format!("JEV decisions HTTP {status}"))
    }
}

impl JevClient {
    pub async fn decide(
        &self,
        state: serde_json::Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<DecisionsResult, LlmError> {
        let mut qjson = serde_json::Map::new();
        for (k, q) in &questions {
            qjson.insert(k.clone(), q.to_json());
        }
        let body = serde_json::json!({
            "model": self.model,
            "state": state,
            "questions": serde_json::Value::Object(qjson),
        });

        let resp = self
            .http
            .post(DECISIONS_ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| LlmError::Transient(format!("JEV decisions 请求失败: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| LlmError::Transient(format!("JEV 响应读取失败: {e}")))?;
        if !status.is_success() {
            return Err(classify_status(status));
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| LlmError::Permanent(format!("JEV 响应非 JSON: {e}")))?;
        let answers_v = v
            .get("answers")
            .and_then(|a| a.as_object())
            .ok_or_else(|| LlmError::Permanent("JEV 响应缺 answers 对象".into()))?;

        let mut answers = BTreeMap::new();
        for (k, av) in answers_v {
            let kind = av.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if let Some(a) = parse_answer(kind, av) {
                answers.insert(k.clone(), a);
            }
        }

        let usage = v.get("usage").cloned().unwrap_or_default();
        Ok(DecisionsResult {
            answers,
            input_tokens: usage
                .get("input_tokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            output_tokens: usage
                .get("output_tokens")
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            cost: usage.get("cost").and_then(|x| x.as_f64()).unwrap_or(0.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_masking() {
        let cfg = JevConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.key_configured());
        assert_eq!(cfg.model, "typesafe/jev-1.13");
        assert!((cfg.reject_threshold - 0.3).abs() < 1e-9);
        let m = masked(&cfg);
        assert_eq!(m["provider"], "openrouter");
        assert!(m.get("api_key_enc").is_none(), "脱敏视图不得含密文");
    }

    #[test]
    fn config_roundtrip_json() {
        let cfg = JevConfig {
            enabled: true,
            api_key_enc: Some("ab12".into()),
            ..JevConfig::default()
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: JevConfig = serde_json::from_str(&s).unwrap();
        assert!(back.enabled);
        assert_eq!(back.api_key_enc.as_deref(), Some("ab12"));
    }

    #[test]
    fn resolve_disabled_returns_none() {
        let cfg = JevConfig::default();
        let cipher = KeyCipher::from_hex_master(&"a".repeat(64)).unwrap();
        let got = resolve(&cfg, &cipher, reqwest::Client::new()).unwrap();
        assert!(got.is_none(), "未启用必须降级 None（调用方直通）");
    }

    #[test]
    fn resolve_enabled_with_key_roundtrip() {
        let cipher = KeyCipher::from_hex_master(&"a".repeat(64)).unwrap();
        let enc = encrypt_key("sk-or-test", &cipher).unwrap();
        let cfg = JevConfig {
            enabled: true,
            api_key_enc: Some(enc),
            ..JevConfig::default()
        };
        let client = resolve(&cfg, &cipher, reqwest::Client::new())
            .unwrap()
            .expect("enabled+key 应解析出客户端");
        assert_eq!(client.model, "typesafe/jev-1.13");
        assert!((client.reject_threshold - 0.3).abs() < 1e-9);
    }

    #[test]
    fn question_json_shape() {
        let mut criteria = BTreeMap::new();
        criteria.insert("a".to_string(), "opt a".to_string());
        let q = Question::Choice {
            instructions: "pick".into(),
            criteria,
        };
        let j = q.to_json();
        assert_eq!(j["type"], "choice");
        assert_eq!(j["criteria"]["a"], "opt a");
        let n = Question::Noul {
            instructions: "yes?".into(),
        };
        assert_eq!(n.to_json()["type"], "noul");
    }

    #[test]
    fn parse_answer_choice_with_probabilities() {
        let v = serde_json::json!({
            "type": "choice", "choice": "user_facts", "confidence": 0.95,
            "probabilities": {"user_facts": 0.96, "transient": 0.0}
        });
        let a = parse_answer("choice", &v).unwrap();
        assert_eq!(a.choice(), Some("user_facts"));
        match &a {
            Answer::Choice { probabilities, .. } => {
                assert!((probabilities["user_facts"] - 0.96).abs() < 1e-9);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn parse_answer_noul_and_unknown_kind_dropped() {
        let a = parse_answer("noul", &serde_json::json!({"type":"noul","noul":0.71})).unwrap();
        assert!((a.noul().unwrap() - 0.71).abs() < 1e-9);
        assert!(parse_answer("score", &serde_json::json!({"type":"score"})).is_none());
    }

    #[test]
    fn decisions_full_response_shape() {
        // cookbook 捕获的真实响应形状
        let v = serde_json::json!({
            "model": "typesafe/jev-1.13",
            "answers": {
                "verdict": {"type": "choice", "choice": "new", "confidence": 1.0},
                "guard": {"type": "noul", "noul": 0.93}
            },
            "usage": {"input_tokens": 621, "output_tokens": 179, "cost": 0.000026082}
        });
        let answers_v = v.get("answers").unwrap().as_object().unwrap();
        let mut answers = BTreeMap::new();
        for (k, av) in answers_v {
            if let Some(a) = parse_answer(av.get("type").and_then(|t| t.as_str()).unwrap_or(""), av)
            {
                answers.insert(k.clone(), a);
            }
        }
        assert_eq!(answers.len(), 2);
        assert_eq!(answers["verdict"].choice(), Some("new"));
        assert!((answers["guard"].noul().unwrap() - 0.93).abs() < 1e-9);
        let usage = v.get("usage").unwrap();
        assert!(usage["cost"].as_f64().unwrap() > 0.0);
    }
}
