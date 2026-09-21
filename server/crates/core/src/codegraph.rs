//! CodeGraph 域门面——api 经此访问 cg-bridge（Q9 收敛）。
//!
//! 依赖方向：api → core → cg-bridge。

pub use engram_cg_bridge::{
    CG_VERSION_PIN, CgBridge, CgError, CgProjectDto, CliStatus, QueryKind, cli_fix_hint,
};
