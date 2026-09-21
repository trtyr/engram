//! CodeGraph 桥：包装 colbymchenry/codegraph CLI（子进程 + --json）。
//! 不解析任何代码——只做注册/同步/查询代理与错误归一。
//!
//! 设计文档：topics/codegraph-bridge.md；决策 D0005（版本 pin 1.5.0）。
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

pub mod bridge;
/// 文件级初布局（d3-force 同款力律 + Barnes-Hut；index/sync 收尾算好落 layout.json）
pub mod layout;

pub use bridge::{
    CG_VERSION_PIN, CgBridge, CgError, CgProjectDto, CliStatus, QueryKind, cli_fix_hint,
    index_usable, register_handlers,
};
