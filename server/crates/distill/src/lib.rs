//! 蒸馏管道：L0→L1（extract/arbitrate）→L2（organize）→L3（persona）+ consolidate。
//!
//! 设计文档：docs/plantree/plans/engram-platform/topics/distill-pipeline.md
//! 链式执行：每阶段独立 job，成功后显式入队下游（ID 链通过 payload 传递）。

pub mod arbitrate;
pub mod chain;
pub mod consolidate;
pub mod extract;
pub mod llm_port;
pub mod organize;
pub mod persona;
pub mod prompts;
pub mod reembed;

pub use chain::{gateway_llm, register_handlers, trigger_auto_extract};
pub use llm_port::{DistillLlm, GatewayLlm};
