//! CodeGraph 桥：包装 colbymchenry/codegraph CLI（子进程 + --json）。
//! 不解析任何代码——只做注册/同步/查询代理与错误归一。
//!
//! 设计文档：topics/codegraph-bridge.md；决策 D0005（版本 pin 1.5.0）。

pub mod bridge;

pub use bridge::{CgBridge, CgError, CgProjectDto, CliStatus, QueryKind, register_handlers};
