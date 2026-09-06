//! 项目记忆域 MCP 工具集成测试：JSON-RPC 全链路（真 PG + 完整 router + rmcp）。
//!
//! 覆盖：工具清单与 scope 过滤、建/查/改/删全旅程、位置与文档 CRUD、
//! 补丁式更新语义、分类校验、改名冲突、批量删除、级联删除、工具停用开关。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{
    app, create_key, expect_result, login_token, mcp_call_json, mcp_initialize, mcp_rpc, rpc,
};
use tower::util::ServiceExt;

const PROJECT_TOOLS: [&str; 15] = [
    "project_types",
    "project_list",
    "project_get",
    "project_create",
    "project_update",
    "project_delete",
    "project_batch_delete",
    "project_location_add",
    "project_location_update",
    "project_location_delete",
    "project_doc_add",
    "project_doc_get",
    "project_doc_search",
    "project_doc_update",
    "project_doc_delete",
];

async fn tool_names(app: &Router, auth: &str) -> Vec<String> {
    let (_, v) = mcp_rpc(app, auth, rpc(9, "tools/list", json!({}))).await;
    expect_result(&v, "tools/list")["tools"]
        .as_array()
        .expect("tools 数组")
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

#[tokio::test]
async fn project_tools_listed_with_annotations() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;

    let info = mcp_initialize(&app, &key).await;
    assert!(
        info["instructions"]
            .as_str()
            .unwrap_or("")
            .contains("project_get"),
        "instructions 应包含项目记忆用法：{}",
        info["instructions"]
    );

    let names = tool_names(&app, &key).await;
    for expected in PROJECT_TOOLS {
        assert!(
            names.contains(&expected.to_string()),
            "缺少 {expected}：{names:?}"
        );
    }
    // project scope 的 key 只见 project_*（scope 过滤），memory_* 不出现
    assert!(
        !names.iter().any(|n| n.starts_with("memory_")),
        "project scope 不应看到 memory 工具：{names:?}"
    );

    // 只读/破坏性注解与实现一致
    let (_, v) = mcp_rpc(&app, &key, rpc(10, "tools/list", json!({}))).await;
    let tools = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .unwrap()
        .clone();
    let ann = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("缺工具 {name}"))["annotations"]
            .clone()
    };
    assert_eq!(ann("project_list")["readOnlyHint"], json!(true));
    assert_eq!(ann("project_get")["readOnlyHint"], json!(true));
    assert_eq!(ann("project_doc_get")["readOnlyHint"], json!(true));
    assert_eq!(ann("project_create")["readOnlyHint"], json!(false));
    assert_eq!(ann("project_delete")["destructiveHint"], json!(true));
    assert_eq!(ann("project_doc_delete")["destructiveHint"], json!(true));
    assert_eq!(ann("project_batch_delete")["destructiveHint"], json!(true));
}

#[tokio::test]
async fn project_types_template() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let types = mcp_call_json(&app, &key, "project_types", json!({})).await;
    let arr = types.as_array().expect("类型数组");
    let dev = arr.iter().find(|t| t["type"] == "dev").expect("dev 类型");
    assert_eq!(dev["label"], "开发");
    assert_eq!(
        dev["default_categories"],
        json!(["后端", "前端", "测试", "部署", "规划"])
    );
    let research = arr
        .iter()
        .find(|t| t["type"] == "research")
        .expect("research 类型");
    assert_eq!(
        research["default_categories"],
        json!(["待查", "线索", "资料", "结论", "疑点", "证伪"])
    );
}

/// 全旅程：建 → 登记 → 写文档 → 查 → 改（补丁式）→ 清理。
#[tokio::test]
async fn project_full_journey() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project", "memory"]).await;
    mcp_initialize(&app, &key).await;

    // 建项目（dev）→ 类型模板分类被复制
    let created = mcp_call_json(
        &app,
        &key,
        "project_create",
        json!({"name": "Engram 项目记忆 MCP", "type": "dev", "description": "把项目记忆做成 MCP"}),
    )
    .await;
    assert_eq!(created["status"], "active", "新项目默认进行中");
    assert_eq!(
        created["categories"],
        json!(["后端", "前端", "测试", "部署", "规划"]),
        "dev 类型应预置五分类"
    );
    let project_id = created["id"].as_str().unwrap().to_string();

    // 同名再建 → JSON-RPC 错误层给出冲突说明
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            3,
            "tools/call",
            json!({"name": "project_create", "arguments": {"name": "Engram 项目记忆 MCP", "type": "dev"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已存在"),
        "撞名应报 Conflict：{v}"
    );

    // 未知类型 → 参数错误
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            4,
            "tools/call",
            json!({"name": "project_create", "arguments": {"name": "x", "type": "bogus"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("未知项目类型"),
        "未知类型应报 BadRequest：{v}"
    );

    // 登记两个位置
    let loc = mcp_call_json(
        &app,
        &key,
        "project_location_add",
        json!({
            "project_name": "Engram 项目记忆 MCP",
            "ip": "127.0.0.1", "host": "MacBook Pro", "os": "macOS",
            "path": "/Users/trtyr/Documents/Code/Rust/agent-memory-projectmcp",
            "purpose": "开发"
        }),
    )
    .await;
    assert_eq!(loc["host"], "MacBook Pro");
    let loc_id = loc["id"].as_str().unwrap().to_string();

    // 写文档（按名定位项目）
    let doc = mcp_call_json(
        &app,
        &key,
        "project_doc_add",
        json!({
            "project_name": "Engram 项目记忆 MCP",
            "category": "后端", "title": "MCP 工具设计",
            "content": "# 工具面\n\n14 个 project_* 工具并入 /mcp。"
        }),
    )
    .await;
    assert_eq!(doc["category"], "后端");
    let doc_id = doc["id"].as_str().unwrap().to_string();

    // 同分类同标题 → 冲突
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            5,
            "tools/call",
            json!({"name": "project_doc_add", "arguments": {
                "project_id": project_id, "category": "后端",
                "title": "MCP 工具设计", "content": "重复"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已存在"),
        "同分类同标题应报冲突：{v}"
    );

    // 未登记分类 → 参数错误并提示现有分类（防孤儿分类）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            6,
            "tools/call",
            json!({"name": "project_doc_add", "arguments": {
                "project_id": project_id, "category": "后段", "title": "t", "content": ""}}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("不在项目分类里"), "未登记分类应报错：{v}");
    assert!(msg.contains("后端"), "错误应列出现有分类：{msg}");

    // project_get 按 id 取详情（默认索引模式）：位置与文档都在，正文只给字符数
    let detail = mcp_call_json(&app, &key, "project_get", json!({"project_id": project_id})).await;
    assert_eq!(detail["name"], "Engram 项目记忆 MCP");
    assert_eq!(detail["locations"].as_array().unwrap().len(), 1);
    assert_eq!(detail["docs"].as_array().unwrap().len(), 1);
    assert!(
        detail["docs"][0].get("content").is_none(),
        "索引模式不应带正文：{}",
        detail["docs"][0]
    );
    assert_eq!(
        detail["docs"][0]["content_chars"],
        json!(
            "# 工具面\n\n14 个 project_* 工具并入 /mcp。"
                .chars()
                .count()
        )
    );
    assert_eq!(detail["docs"][0]["title"], "MCP 工具设计");

    // include_content=true：无损全量
    let full = mcp_call_json(
        &app,
        &key,
        "project_get",
        json!({"project_id": project_id, "include_content": true}),
    )
    .await;
    assert_eq!(
        full["docs"][0]["content"],
        json!("# 工具面\n\n14 个 project_* 工具并入 /mcp。")
    );

    // project_get 按名取详情（名字寻址）
    let by_name = mcp_call_json(
        &app,
        &key,
        "project_get",
        json!({"project_name": "Engram 项目记忆 MCP"}),
    )
    .await;
    assert_eq!(by_name["id"], json!(project_id));

    // 文档补丁式更新：只传 content，category/title 不动
    let updated_doc = mcp_call_json(
        &app,
        &key,
        "project_doc_update",
        json!({"doc_id": doc_id, "content": "# 工具面\n\n14 个工具。\n\n## 更新\n补丁式更新可用。"}),
    )
    .await;
    assert_eq!(updated_doc["title"], "MCP 工具设计", "未传 title 不应改变");
    assert_eq!(updated_doc["category"], "后端");
    assert!(
        updated_doc["content"]
            .as_str()
            .unwrap()
            .contains("补丁式更新可用")
    );

    // 文档移到未登记分类 → 报错
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            7,
            "tools/call",
            json!({"name": "project_doc_update",
            "arguments": {"doc_id": doc_id, "category": "前端x"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不在项目分类里"),
        "移到未登记分类应报错：{v}"
    );

    // 位置补丁式更新：只传 path
    let upd_loc = mcp_call_json(
        &app,
        &key,
        "project_location_update",
        json!({"location_id": loc_id, "path": "/new/path"}),
    )
    .await;
    assert_eq!(upd_loc["path"], "/new/path");
    assert_eq!(upd_loc["host"], "MacBook Pro", "未传 host 不应改变");

    // 项目补丁式更新：只改状态与描述，分类保持
    let upd = mcp_call_json(
        &app,
        &key,
        "project_update",
        json!({"project_id": project_id, "status": "paused", "description": "第一版完成，暂停"}),
    )
    .await;
    assert_eq!(upd["status"], "paused");
    assert_eq!(upd["description"], "第一版完成，暂停");
    assert_eq!(
        upd["categories"],
        json!(["后端", "前端", "测试", "部署", "规划"]),
        "补丁式更新不应动分类"
    );

    // 追加分类：替换式带全量
    let upd2 = mcp_call_json(
        &app,
        &key,
        "project_update",
        json!({"project_id": project_id, "categories": ["后端", "前端", "测试", "规划", "运维"]}),
    )
    .await;
    assert_eq!(
        upd2["categories"],
        json!(["后端", "前端", "测试", "规划", "运维"])
    );

    // 改名：new_name；旧名寻址失效、新名可用
    mcp_call_json(
        &app,
        &key,
        "project_update",
        json!({"project_id": project_id, "new_name": "Engram MCP"}),
    )
    .await;
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            8,
            "tools/call",
            json!({"name": "project_get",
            "arguments": {"project_name": "Engram 项目记忆 MCP"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不存在"),
        "旧名寻址应 NotFound：{v}"
    );
    let renamed = mcp_call_json(
        &app,
        &key,
        "project_get",
        json!({"project_name": "Engram MCP"}),
    )
    .await;
    assert_eq!(renamed["id"], json!(project_id));

    // 列表 + 类型过滤
    let listed = mcp_call_json(&app, &key, "project_list", json!({"type": "dev"})).await;
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == json!(project_id))
    );
    let none = mcp_call_json(&app, &key, "project_list", json!({"type": "research"})).await;
    assert_eq!(none.as_array().unwrap().len(), 0, "research 过滤应为空");

    // 删位置、删文档
    mcp_call_json(
        &app,
        &key,
        "project_location_delete",
        json!({"location_id": loc_id}),
    )
    .await;
    mcp_call_json(&app, &key, "project_doc_delete", json!({"doc_id": doc_id})).await;
    let detail2 = mcp_call_json(&app, &key, "project_get", json!({"project_id": project_id})).await;
    assert_eq!(detail2["locations"].as_array().unwrap().len(), 0);
    assert_eq!(detail2["docs"].as_array().unwrap().len(), 0);

    // 删项目 → 级联（本例已无子行）+ 再查 NotFound
    mcp_call_json(
        &app,
        &key,
        "project_delete",
        json!({"project_id": project_id}),
    )
    .await;
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            11,
            "tools/call",
            json!({"name": "project_get", "arguments": {"project_id": project_id}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不存在"),
        "删除后查询应 NotFound：{v}"
    );
}

/// 改名撞名：服务层修复后应给 Conflict 语义而非 500。
#[tokio::test]
async fn project_rename_conflict_is_conflict() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let a = mcp_call_json(
        &app,
        &key,
        "project_create",
        json!({"name": "项目A", "type": "dev"}),
    )
    .await;
    mcp_call_json(
        &app,
        &key,
        "project_create",
        json!({"name": "项目B", "type": "dev"}),
    )
    .await;

    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            2,
            "tools/call",
            json!({"name": "project_update",
            "arguments": {"project_id": a["id"], "new_name": "项目B"}}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("已被项目"), "改名撞名应给 Conflict 说明：{v}");

    // 空名 / 纯空白名 → BadRequest
    for bad in ["", "   "] {
        let (_, v) = mcp_rpc(
            &app,
            &key,
            rpc(
                3,
                "tools/call",
                json!({"name": "project_update",
                "arguments": {"project_id": a["id"], "new_name": bad}}),
            ),
        )
        .await;
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("不能为空"),
            "空名应被拒：{v}"
        );
    }

    // 空名建项目同样被拒
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            4,
            "tools/call",
            json!({"name": "project_create",
            "arguments": {"name": "", "type": "dev"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不能为空"),
        "空名建项目应被拒：{v}"
    );
}

/// 批量删除：deleted/failed 分账；级联删除位置与文档。
#[tokio::test]
async fn project_batch_delete_and_cascade() {
    let (app, pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let p1 = mcp_call_json(
        &app,
        &key,
        "project_create",
        json!({"name": "调研一", "type": "research"}),
    )
    .await;
    let p2 = mcp_call_json(
        &app,
        &key,
        "project_create",
        json!({"name": "调研二", "type": "research"}),
    )
    .await;
    assert_eq!(
        p1["categories"],
        json!(["待查", "线索", "资料", "结论", "疑点", "证伪"]),
        "research 类型应预置六分类"
    );

    // p1 挂位置和文档，验证项目删除级联
    let loc = mcp_call_json(
        &app,
        &key,
        "project_location_add",
        json!({"project_id": p1["id"], "ip": "10.0.0.2", "host": "tencent-beijing",
               "os": "Ubuntu", "path": "/srv/engram", "purpose": "部署"}),
    )
    .await;
    let doc = mcp_call_json(
        &app,
        &key,
        "project_doc_add",
        json!({"project_id": p1["id"], "category": "待查", "title": "问题清单", "content": "1. …"}),
    )
    .await;

    let ids = [
        p1["id"].as_str().unwrap(),
        p2["id"].as_str().unwrap(),
        "00000000-0000-0000-0000-000000000000",
    ];
    let result = mcp_call_json(&app, &key, "project_batch_delete", json!({"ids": ids})).await;
    assert_eq!(result["deleted"], json!(2));
    assert_eq!(
        result["failed"],
        json!(["00000000-0000-0000-0000-000000000000"])
    );

    // 级联验证：直接查表——位置/文档行应随项目一起没了
    let loc_id = uuid::Uuid::parse_str(loc["id"].as_str().unwrap()).unwrap();
    let doc_id = uuid::Uuid::parse_str(doc["id"].as_str().unwrap()).unwrap();
    let pool = sqlx::PgPool::connect(&support::connection_url(&pg).await.unwrap())
        .await
        .unwrap();
    let locs: i64 = sqlx::query_scalar("SELECT count(*) FROM project_locations WHERE id = $1")
        .bind(loc_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let docs: i64 = sqlx::query_scalar("SELECT count(*) FROM project_docs WHERE id = $1")
        .bind(doc_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((locs, docs), (0, 0), "项目删除应级联清掉位置与文档");
    pool.close().await;
}

/// scope 分权：project scope 的 key 调 memory 工具被拒；memory-only key 看不到 project 工具。
#[tokio::test]
async fn project_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let proj_key = create_key(&app, &admin, &["project"]).await;
    mcp_initialize(&app, &proj_key).await;

    // project scope 调 memory 工具 → 拒
    let (_, v) = mcp_rpc(
        &app,
        &proj_key,
        rpc(
            2,
            "tools/call",
            json!({"name": "memory_search", "arguments": {"query": "x"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("memory scope"),
        "project key 调 memory 工具应被拒：{v}"
    );

    // memory-only key：tools/list 不见 project_*，调用被拒
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let names = tool_names(&app, &mem_key).await;
    assert!(
        !names.iter().any(|n| n.starts_with("project_")),
        "memory-only key 不应看到 project 工具：{names:?}"
    );
    let (_, v) = mcp_rpc(
        &app,
        &mem_key,
        rpc(
            3,
            "tools/call",
            json!({"name": "project_create", "arguments": {"name": "x", "type": "dev"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("project scope"),
        "memory key 调 project 工具应被拒：{v}"
    );

    // 双 scope key 两边都可用
    let both = create_key(&app, &admin, &["memory", "project"]).await;
    let names = tool_names(&app, &both).await;
    assert!(names.iter().any(|n| n == "project_create"));
    assert!(names.iter().any(|n| n == "memory_context"));

    // 缺定位参数 → 参数错误（不是内部错误）
    let (_, v) = mcp_rpc(
        &app,
        &both,
        rpc(
            4,
            "tools/call",
            json!({"name": "project_get", "arguments": {}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("project_id 或 project_name"),
        "缺定位应提示二选一：{v}"
    );

    // 坏 UUID → 参数错误
    let (_, v) = mcp_rpc(
        &app,
        &both,
        rpc(
            5,
            "tools/call",
            json!({"name": "project_get", "arguments": {"project_id": "not-a-uuid"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("UUID"),
        "坏 UUID 应报参数错误：{v}"
    );
}

/// 管理台同源：build_info 应含 project 域；工具停用开关对 project_* 同样生效。
#[tokio::test]
async fn project_tools_admin_info_and_toggle() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["project"]).await;

    // GET /settings/mcp：project 域工具与 memory 域同源展示
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/mcp")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&body).unwrap();
    let tools = info["tools"].as_array().expect("工具清单");
    let project_tools: Vec<&Value> = tools.iter().filter(|t| t["domain"] == "project").collect();
    assert_eq!(
        project_tools.len(),
        15,
        "project 域应自动分组 15 个工具：{tools:?}"
    );

    // 停用 project_delete：tools/list 隐身 + call 拒绝
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/settings/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::from(r#"{"disabled_tools":["project_delete"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let names = tool_names(&app, &key).await;
    assert!(
        !names.contains(&"project_delete".to_string()),
        "停用后不应出现在 tools/list"
    );
    let other_count = names.len();
    assert_eq!(other_count, 14, "其余 14 个应仍在：{names:?}");

    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(2, "tools/call", json!({"name": "project_delete", "arguments": {"project_id": "00000000-0000-0000-0000-000000000000"}})),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已停用"),
        "停用工具调用应报错：{v}"
    );

    // 未知工具名 400（校验名单覆盖 project_*）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/settings/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::from(r#"{"disabled_tools":["project_bogus"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 恢复全开
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/settings/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::from(r#"{"disabled_tools":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let names = tool_names(&app, &key).await;
    assert!(names.contains(&"project_delete".to_string()));
}

/// 未传分类必填字段等 schema 缺参：JSON-RPC 参数错误而非 panic。
#[tokio::test]
async fn project_doc_add_missing_fields_rejected() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    // 缺 category（schema 必填）→ rmcp 参数反序列化失败，isError=true
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            1,
            "tools/call",
            json!({"name": "project_doc_add",
            "arguments": {"project_name": "不存在", "title": "t", "content": "c"}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call project_doc_add");
    assert!(
        out["isError"].as_bool().unwrap_or(false),
        "缺必填参数应 isError：{out}"
    );
    assert!(
        out["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("missing field"),
        "应提示缺字段：{out}"
    );
}

/// 精确寻址读设计：索引模式 → 搜索定位行号 → 区间精读；全文模式无损；
/// 行号边界校验；分类过滤。全程无截断。
#[tokio::test]
async fn project_precise_addressing_read() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let p = mcp_call_json(
        &app,
        &key,
        "project_create",
        json!({"name": "寻址读", "type": "dev"}),
    )
    .await;
    let pid = p["id"].as_str().unwrap().to_string();

    // 长文档：12 行，含特征词
    let progress = (1..=12)
        .map(|i| {
            if i == 7 {
                "第七行提到 streamable http 传输".to_string()
            } else {
                format!("第{i}行普通内容")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let doc = mcp_call_json(
        &app,
        &key,
        "project_doc_add",
        json!({"project_id": pid, "category": "后端", "title": "进度", "content": progress}),
    )
    .await;
    let doc_id = doc["id"].as_str().unwrap().to_string();
    let other = mcp_call_json(
        &app,
        &key,
        "project_doc_add",
        json!({"project_id": pid, "category": "规划", "title": "结论", "content": "结论一：MCP 并入 /mcp\n结论二：scope 分权"}),
    )
    .await;

    // 索引模式：无正文，有 content_chars
    let idx = mcp_call_json(&app, &key, "project_get", json!({"project_id": pid})).await;
    let docs = idx["docs"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    for d in docs {
        assert!(d.get("content").is_none(), "索引模式不带正文：{d}");
        assert!(d["content_chars"].as_i64().unwrap() > 0, "{d}");
        assert!(d.get("id").is_some() && d.get("title").is_some(), "{d}");
    }

    // 全量模式：无损拿回原文
    let full = mcp_call_json(
        &app,
        &key,
        "project_get",
        json!({"project_id": pid, "include_content": true}),
    )
    .await;
    assert_eq!(full["docs"].as_array().unwrap().len(), 2);
    assert_eq!(full["docs"][0]["content"], json!(progress));

    // 分类过滤
    let filtered = mcp_call_json(
        &app,
        &key,
        "project_get",
        json!({"project_id": pid, "category": "规划"}),
    )
    .await;
    let fd = filtered["docs"].as_array().unwrap();
    assert_eq!(fd.len(), 1);
    assert_eq!(fd[0]["title"], "结论");

    // 搜索定位：命中行号 + 原文行（大小写不敏感）
    let hits = mcp_call_json(
        &app,
        &key,
        "project_doc_search",
        json!({"project_id": pid, "query": "Streamable HTTP"}),
    )
    .await;
    let arr = hits.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{hits}");
    assert_eq!(arr[0]["doc_id"], json!(doc_id));
    assert_eq!(arr[0]["line"], 7);
    assert_eq!(arr[0]["text"], "第七行提到 streamable http 传输");

    // 搜索跨行：同一篇文档多行命中
    let multi = mcp_call_json(
        &app,
        &key,
        "project_doc_search",
        json!({"project_name": "寻址读", "query": "结论"}),
    )
    .await;
    let mh = multi.as_array().unwrap();
    assert_eq!(mh.len(), 2, "{multi}");
    assert_eq!(mh[0]["line"], 1);
    assert_eq!(mh[1]["line"], 2);

    // 区间精读 5-9 行：恒带行号前缀，端点含入
    let range = mcp_call_json(
        &app,
        &key,
        "project_doc_get",
        json!({"doc_id": doc_id, "start_line": 5, "end_line": 9}),
    )
    .await;
    assert_eq!(range["total_lines"], 12);
    assert_eq!(range["start_line"], 5);
    assert_eq!(range["end_line"], 9);
    let content = range["content"].as_str().unwrap();
    for l in 5..=9 {
        assert!(content.contains(&format!("{l}: ")), "缺 {l} 行：{content}");
    }
    assert!(content.contains("5: 第5行普通内容"), "{content}");
    assert!(content.contains("9: 第9行普通内容"), "{content}");
    assert!(!content.contains("1: 第1行"), "区间外行不应出现：{content}");
    assert!(content.contains("7: 第七行"));

    // 区间开右端：start=11 到末尾
    let tail = mcp_call_json(
        &app,
        &key,
        "project_doc_get",
        json!({"doc_id": doc_id, "start_line": 11}),
    )
    .await;
    assert_eq!(tail["end_line"], 12);
    assert!(tail["content"].as_str().unwrap().contains("12: 第12行"));

    // 全文模式：无损原文 + total_lines；with_line_numbers 加行号
    let whole = mcp_call_json(&app, &key, "project_doc_get", json!({"doc_id": doc_id})).await;
    assert_eq!(whole["content"], json!(progress));
    assert_eq!(whole["total_lines"], 12);
    let numbered = mcp_call_json(
        &app,
        &key,
        "project_doc_get",
        json!({"doc_id": doc_id, "with_line_numbers": true}),
    )
    .await;
    assert!(
        numbered["content"]
            .as_str()
            .unwrap()
            .starts_with("1: 第1行")
    );

    // 边界与参数错误
    for bad in [
        json!({"doc_id": doc_id, "start_line": 0}),
        json!({"doc_id": doc_id, "start_line": 9, "end_line": 3}),
    ] {
        let (_, v) = mcp_rpc(
            &app,
            &key,
            rpc(
                20,
                "tools/call",
                json!({"name": "project_doc_get", "arguments": bad}),
            ),
        )
        .await;
        assert!(v.get("error").is_some(), "坏区间应报错：{v}");
    }
    // 空检索词
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            21,
            "tools/call",
            json!({"name": "project_doc_search", "arguments": {"project_id": pid, "query": "  "}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不能为空"),
        "空检索词应报错：{v}"
    );

    // 精读另一篇 + limit 上限
    let limited = mcp_call_json(
        &app,
        &key,
        "project_doc_search",
        json!({"project_id": pid, "query": "行", "limit": 2}),
    )
    .await;
    assert_eq!(limited.as_array().unwrap().len(), 2);

    // 覆盖 other 引用，避免未使用告警
    assert_eq!(other["title"], "结论");
}
