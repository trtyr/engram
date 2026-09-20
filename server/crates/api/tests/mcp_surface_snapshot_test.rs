//! MCP 工具面快照（架构拆分的安全网）：
//!
//! 归档 `initialize` 的 instructions + `tools/list` 的完整工具表，逐字节比对。
//! **拆分为纯搬移的证明**：拆分前后本测试必须原样通过；任何工具名、action 文档、
//! 参数 schema、作用域提示文本的漂移都会让它失败。
//!
//! 基线更新（只在有意变更工具面时使用）：
//! `UPDATE_GOLDEN=1 cargo test -p engram-api --test mcp_surface_snapshot_test`

mod support;

use serde_json::Value;
use support::{app, expect_result, login_token, mcp_initialize, mcp_rpc, rpc};

const GOLDEN_REL: &str = "tests/golden/mcp_surface.json";

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_REL)
}

/// 规范序列化：serde_json 的 Value 采用有序映射，输出即稳定。
fn canonical(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("工具面应可序列化")
}

#[tokio::test]
async fn mcp_tool_surface_is_byte_stable() {
    let (app, _pg) = app().await;
    // 管理员会话 → 全作用域，tools/list 不过滤，快照覆盖完整工具面。
    let tok = login_token(&app).await;

    let init = mcp_initialize(&app, &tok).await;
    let (status, list_v) = mcp_rpc(&app, &tok, rpc(2, "tools/list", serde_json::json!({}))).await;
    assert_eq!(status, 200, "tools/list 应 200");
    let list = expect_result(&list_v, "tools/list");

    let surface = serde_json::json!({
        "protocol_version": init.get("protocolVersion").cloned().unwrap_or(Value::Null),
        "instructions": init.get("instructions").cloned().unwrap_or(Value::Null),
        "tools": list.get("tools").cloned().unwrap_or(Value::Null),
    });
    let got = canonical(&surface);

    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().expect("golden 目录")).expect("建目录");
        std::fs::write(&path, format!("{got}\n")).expect("写基线");
        eprintln!("已写入工具面基线：{}", path.display());
        return;
    }

    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "缺少工具面基线 {}：{e}（首次可用 UPDATE_GOLDEN=1 生成）",
            path.display()
        )
    });
    assert_eq!(
        got.trim_end(),
        want.trim_end(),
        "工具面与基线不一致——架构拆分必须零行为变化（确属有意变更才更新基线）"
    );
}
