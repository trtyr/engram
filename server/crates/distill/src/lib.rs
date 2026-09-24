//! 蒸馏管道：L0→L1（extract/arbitrate）→L2（organize）→L3（persona）+ consolidate。
//!
//! 设计文档：engram projects 域（主题：distill-pipeline）
//! 链式执行：每阶段独立 job，成功后显式入队下游（ID 链通过 payload 传递）。
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

pub mod arbitrate;
pub mod chain;
pub mod consolidate;
pub mod entity_portraits;
pub mod extract;
pub mod extract_model;
pub mod llm_port;
pub mod organize;
pub mod persona;
pub mod prompts;
pub mod reembed;
pub mod rhythm;
pub mod scenario_converge;

pub use chain::{gateway_llm, register_handlers, trigger_auto_extract};
pub use llm_port::{DistillLlm, GatewayLlm};
pub use rhythm::{RhythmConfig, bootstrap, load_config, register_rhythm, save_config};
