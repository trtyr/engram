//! 任务系统：PG-backed 队列（SKIP LOCKED 抢占）、重试、链式入队、事件流。
//! 一切长操作（蒸馏/摄取/ingest/同步）都走 job。
//!
//! 设计文档：engram projects 域（主题：jobs-system）
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

pub mod admin;
pub mod queue;
pub mod runner;
pub mod types;

pub use queue::JobQueue;
pub use runner::{HandlerFn, JobContext, Runner, RunnerConfig, RunnerHandle};
pub use types::{FailOutcome, Job, JobError, JobEvent, JobStatus, JobTemplate};
