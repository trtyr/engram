//! R 报告 P1-10：projects doc_patch 行级补丁（replace/insert/delete）集成测试。

mod support;

use axum::Router;
use serde_json::{Value, json};
use support::{app, create_key, expect_result, login_token, mcp_rpc, rpc};

fn call(id: i64, tool: &str, action: &str, args: Value) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert("action".into(), json!(action));
    if let Value::Object(map) = args {
        for (k, v) in map {
            arguments.insert(k, v);
        }
    }
    rpc(
        id,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    )
}

fn out_json(v: &Value, what: &str) -> Value {
    let out = expect_result(v, what);
    serde_json::from_str(out["content"][0]["text"].as_str().expect("文本内容")).expect("JSON")
}

#[tokio::test]
async fn project_doc_patch_replace_insert_delete() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;

    // 建项目 + 5 行文档
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "projects",
            "create",
            json!({"name": "zz-patch", "type": "dev"}),
        ),
    )
    .await;
    let project_id = out_json(&v, "create")["id"].as_str().unwrap().to_string();
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "projects",
            "doc_add",
            json!({
                "project_id": project_id, "category": "后端", "title": "补丁靶文档",
                "content": "L1\nL2\nL3\nL4\nL5",
            }),
        ),
    )
    .await;
    let doc = out_json(&v, "doc_add");
    assert!(
        doc["content_chars"].is_i64() && doc.get("content").is_none(),
        "P0-1：doc_add 瘦身：{doc}"
    );
    let doc_id = doc["id"].as_str().unwrap().to_string();

    let read = |app: &Router, key: &str, id: i64, doc_id: &str| {
        let app = app.clone();
        let key = key.to_string();
        let doc_id = doc_id.to_string();
        async move {
            let (_, v) = mcp_rpc(
                &app,
                &key,
                call(
                    id,
                    "projects",
                    "doc_get",
                    json!({"doc_id": doc_id, "with_line_numbers": true}),
                ),
            )
            .await;
            out_json(&v, "doc_get")["content"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };

    // replace：2-3 行换成两行新文本
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            3,
            "projects",
            "doc_patch",
            json!({"doc_id": doc_id, "start_line": 2, "end_line": 3, "content": "X\nY"}),
        ),
    )
    .await;
    let r = out_json(&v, "patch replace");
    assert_eq!(r["patched"]["mode"], "replace");
    assert_eq!(r["total_lines"], 5);
    assert_eq!(
        read(&app, &key, 20, &doc_id).await,
        "1: L1\n2: X\n3: Y\n4: L4\n5: L5"
    );

    // insert：在第 1 行前插入（文档变 6 行）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            4,
            "projects",
            "doc_patch",
            json!({"doc_id": doc_id, "start_line": 1, "end_line": 1, "mode": "insert", "content": "TOP"}),
        ),
    )
    .await;
    expect_result(&v, "patch insert");
    assert_eq!(
        read(&app, &key, 21, &doc_id).await,
        "1: TOP\n2: L1\n3: X\n4: Y\n5: L4\n6: L5"
    );

    // insert：start_line = total+1 追加到末尾
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            5,
            "projects",
            "doc_patch",
            json!({"doc_id": doc_id, "start_line": 7, "end_line": 7, "mode": "insert", "content": "END"}),
        ),
    )
    .await;
    expect_result(&v, "patch append");
    assert!(read(&app, &key, 22, &doc_id).await.ends_with("7: END"));

    // delete：删 3-4 行
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            6,
            "projects",
            "doc_patch",
            json!({"doc_id": doc_id, "start_line": 3, "end_line": 4, "mode": "delete"}),
        ),
    )
    .await;
    expect_result(&v, "patch delete");
    assert_eq!(
        read(&app, &key, 23, &doc_id).await,
        "1: TOP\n2: L1\n3: L4\n4: L5\n5: END"
    );

    // 校验：replace 缺 content / end_line 超界 / mode 非法 —— 全部响亮拒绝
    for (id, args, expect) in [
        (
            7,
            json!({"doc_id": doc_id, "start_line": 1, "end_line": 1}),
            "需要传 content",
        ),
        (
            8,
            json!({"doc_id": doc_id, "start_line": 1, "end_line": 99, "content": "x"}),
            "超出文档总行数",
        ),
        (
            9,
            json!({"doc_id": doc_id, "start_line": 1, "end_line": 1, "mode": "nuke", "content": "x"}),
            "replace/insert/delete",
        ),
    ] {
        let (_, v) = mcp_rpc(&app, &key, call(id, "projects", "doc_patch", args)).await;
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains(expect),
            "{id} 应报 {expect}：{v}"
        );
    }
}
