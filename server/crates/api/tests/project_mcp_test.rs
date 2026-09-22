//! 项目记忆域 MCP 集成测试：渐进式发现语法（{"name":"projects","arguments":{"action":…}}）。
//!
//! 覆盖：域工具清单与 scope 过滤、建/查/改/删全旅程、位置与文档 CRUD、
//! 补丁式更新语义、分类校验、改名冲突、批量删除、级联删除、action 级停用开关。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, expect_result, login_token, mcp_initialize, mcp_rpc, rpc};
use tower::util::ServiceExt;

/// 域工具调用参数：action + 平铺参数。
fn call(id: i64, action: &str, args: Value) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert("action".into(), json!(action));
    if let Value::Object(m) = args {
        for (k, v) in m {
            arguments.insert(k, v);
        }
    }
    rpc(
        id,
        "tools/call",
        json!({"name": "projects", "arguments": arguments}),
    )
}

/// 成功路径快捷调用：返回 content[0].text 解析出的 JSON（isError 即 panic）。
async fn act_json(app: &Router, auth: &str, action: &str, args: Value) -> Value {
    let (_, v) = mcp_rpc(app, auth, call(2, action, args)).await;
    let out = expect_result(&v, &format!("projects.{action}"));
    assert!(
        !out["isError"].as_bool().unwrap_or(false),
        "projects.{action} 不应报错：{out}"
    );
    let text = out["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("projects.{action} 应返回文本内容：{out}"));
    serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("projects.{action} 返回应是 JSON：{e}\n{text}"))
}

/// 原始响应快捷调用（错误路径断言用）。
async fn act_raw(app: &Router, auth: &str, id: i64, action: &str, args: Value) -> Value {
    let (_, v) = mcp_rpc(app, auth, call(id, action, args)).await;
    v
}

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
            .contains("projects 域用法"),
        "instructions 应包含项目记忆用法：{}",
        info["instructions"]
    );

    // project scope 的 key 见 projects 域工具 + 跨域 search_all（scope 过滤）
    let mut names = tool_names(&app, &key).await;
    names.sort();
    assert_eq!(
        names,
        vec!["jobs", "projects", "search_all"],
        "project scope 应见 projects 域工具与 search_all（jobs 无域 scope 恒可见）"
    );

    // 描述目录：15 个操作齐备，破坏性操作带标注
    let (_, v) = mcp_rpc(&app, &key, rpc(10, "tools/list", json!({}))).await;
    let description = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "projects")
        .expect("projects 工具应在列")["description"]
        .as_str()
        .unwrap()
        .to_string();
    for action in [
        "- types：",
        "- list：",
        "- get：",
        "- create：",
        "- update：",
        "- delete：",
        "- batch_delete：",
        "- location_add：",
        "- doc_get：",
        "- doc_search：",
        "- doc_delete：",
    ] {
        assert!(
            description.contains(action),
            "目录缺 {action}：{description}"
        );
    }
    assert!(
        description.contains("- delete：【破坏性】"),
        "破坏性操作应标注：{description}"
    );
}

#[tokio::test]
async fn project_types_template() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let types = act_json(&app, &key, "types", json!({})).await;
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
    let created = act_json(
        &app,
        &key,
        "create",
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
    let v = act_raw(
        &app,
        &key,
        3,
        "create",
        json!({"name": "Engram 项目记忆 MCP", "type": "dev"}),
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
    let v = act_raw(
        &app,
        &key,
        4,
        "create",
        json!({"name": "x", "type": "bogus"}),
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
    let loc = act_json(
        &app,
        &key,
        "location_add",
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
    let doc = act_json(
        &app,
        &key,
        "doc_add",
        json!({
            "project_name": "Engram 项目记忆 MCP",
            "category": "后端", "title": "MCP 工具设计",
            "content": "# 工具面\n\n项目域工具并入 /mcp。"
        }),
    )
    .await;
    assert_eq!(doc["category"], "后端");
    let doc_id = doc["id"].as_str().unwrap().to_string();

    // 同分类同标题 → 冲突
    let v = act_raw(
        &app,
        &key,
        5,
        "doc_add",
        json!({
            "project_id": project_id, "category": "后端",
            "title": "MCP 工具设计", "content": "重复"}),
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
    let v = act_raw(
        &app,
        &key,
        6,
        "doc_add",
        json!({
            "project_id": project_id, "category": "后段", "title": "t", "content": ""}),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("不在项目分类里"), "未登记分类应报错：{v}");
    assert!(msg.contains("后端"), "错误应列出现有分类：{msg}");

    // get 按 id 取详情（默认索引模式）：位置与文档都在，正文只给字符数
    let detail = act_json(&app, &key, "get", json!({"project_id": project_id})).await;
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
        json!("# 工具面\n\n项目域工具并入 /mcp。".chars().count())
    );
    assert_eq!(detail["docs"][0]["title"], "MCP 工具设计");

    // include_content=true：无损全量
    let full = act_json(
        &app,
        &key,
        "get",
        json!({"project_id": project_id, "include_content": true}),
    )
    .await;
    assert_eq!(
        full["docs"][0]["content"],
        json!("# 工具面\n\n项目域工具并入 /mcp。")
    );

    // get 按名取详情（名字寻址）
    let by_name = act_json(
        &app,
        &key,
        "get",
        json!({"project_name": "Engram 项目记忆 MCP"}),
    )
    .await;
    assert_eq!(by_name["id"], json!(project_id));

    // 文档补丁式更新：只传 content，category/title 不动
    let updated_doc = act_json(
        &app,
        &key,
        "doc_update",
        json!({"doc_id": doc_id, "content": "# 工具面\n\n域内操作。\n\n## 更新\n补丁式更新可用。"}),
    )
    .await;
    assert_eq!(updated_doc["title"], "MCP 工具设计", "未传 title 不应改变");
    assert_eq!(updated_doc["category"], "后端");
    // P0-1 瘦身：写操作不回显正文，只回 content_chars
    assert!(
        updated_doc["content"].is_null() && updated_doc["content_chars"].is_i64(),
        "update 应回瘦身元数据：{updated_doc}"
    );
    let reread = act_json(
        &app,
        &key,
        "doc_get",
        json!({"doc_id": doc_id, "start_line": 6, "end_line": 6}),
    )
    .await;
    assert!(
        reread["content"]
            .as_str()
            .unwrap()
            .contains("补丁式更新可用"),
        "更新应落库：{reread}"
    );

    // 文档移到未登记分类 → 报错
    let v = act_raw(
        &app,
        &key,
        7,
        "doc_update",
        json!({"doc_id": doc_id, "category": "前端x"}),
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
    let upd_loc = act_json(
        &app,
        &key,
        "location_update",
        json!({"location_id": loc_id, "path": "/new/path"}),
    )
    .await;
    assert_eq!(upd_loc["path"], "/new/path");
    assert_eq!(upd_loc["host"], "MacBook Pro", "未传 host 不应改变");

    // 项目补丁式更新：只改状态与描述，分类保持
    let upd = act_json(
        &app,
        &key,
        "update",
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
    let upd2 = act_json(
        &app,
        &key,
        "update",
        json!({"project_id": project_id, "categories": ["后端", "前端", "测试", "规划", "运维"]}),
    )
    .await;
    assert_eq!(
        upd2["categories"],
        json!(["后端", "前端", "测试", "规划", "运维"])
    );

    // 改名：new_name；旧名寻址失效、新名可用
    act_json(
        &app,
        &key,
        "update",
        json!({"project_id": project_id, "new_name": "Engram MCP"}),
    )
    .await;
    let v = act_raw(
        &app,
        &key,
        8,
        "get",
        json!({"project_name": "Engram 项目记忆 MCP"}),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不存在"),
        "旧名寻址应 NotFound：{v}"
    );
    let renamed = act_json(&app, &key, "get", json!({"project_name": "Engram MCP"})).await;
    assert_eq!(renamed["id"], json!(project_id));

    // 列表 + 类型过滤
    let listed = act_json(&app, &key, "list", json!({"type": "dev"})).await;
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == json!(project_id))
    );
    let none = act_json(&app, &key, "list", json!({"type": "research"})).await;
    assert_eq!(none.as_array().unwrap().len(), 0, "research 过滤应为空");

    // 删位置、删文档
    act_json(
        &app,
        &key,
        "location_delete",
        json!({"location_id": loc_id}),
    )
    .await;
    act_json(&app, &key, "doc_delete", json!({"doc_id": doc_id})).await;
    let detail2 = act_json(&app, &key, "get", json!({"project_id": project_id})).await;
    assert_eq!(detail2["locations"].as_array().unwrap().len(), 0);
    assert_eq!(detail2["docs"].as_array().unwrap().len(), 0);

    // 删项目 → 级联（本例已无子行）+ 再查 NotFound
    act_json(&app, &key, "delete", json!({"project_id": project_id})).await;
    let v = act_raw(&app, &key, 11, "get", json!({"project_id": project_id})).await;
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

    let a = act_json(
        &app,
        &key,
        "create",
        json!({"name": "项目A", "type": "dev"}),
    )
    .await;
    act_json(
        &app,
        &key,
        "create",
        json!({"name": "项目B", "type": "dev"}),
    )
    .await;

    let v = act_raw(
        &app,
        &key,
        2,
        "update",
        json!({"project_id": a["id"], "new_name": "项目B"}),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("已被项目"), "改名撞名应给 Conflict 说明：{v}");

    // 空名 / 纯空白名 → BadRequest
    for bad in ["", "   "] {
        let v = act_raw(
            &app,
            &key,
            3,
            "update",
            json!({"project_id": a["id"], "new_name": bad}),
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
    let v = act_raw(&app, &key, 4, "create", json!({"name": "", "type": "dev"})).await;
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

    let p1 = act_json(
        &app,
        &key,
        "create",
        json!({"name": "调研一", "type": "research"}),
    )
    .await;
    let p2 = act_json(
        &app,
        &key,
        "create",
        json!({"name": "调研二", "type": "research"}),
    )
    .await;
    assert_eq!(
        p1["categories"],
        json!(["待查", "线索", "资料", "结论", "疑点", "证伪"]),
        "research 类型应预置六分类"
    );

    // p1 挂位置和文档，验证项目删除级联
    let loc = act_json(
        &app,
        &key,
        "location_add",
        json!({"project_id": p1["id"], "ip": "10.0.0.2", "host": "tencent-beijing",
               "os": "Ubuntu", "path": "/srv/engram", "purpose": "部署"}),
    )
    .await;
    let doc = act_json(
        &app,
        &key,
        "doc_add",
        json!({"project_id": p1["id"], "category": "待查", "title": "问题清单", "content": "1. …"}),
    )
    .await;

    let ids = [
        p1["id"].as_str().unwrap(),
        p2["id"].as_str().unwrap(),
        "00000000-0000-0000-0000-000000000000",
    ];
    let result = act_json(&app, &key, "batch_delete", json!({"ids": ids})).await;
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

/// scope 分权：project scope 的 key 调 memory 域被拒；memory-only key 看不到 projects。
#[tokio::test]
async fn project_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let proj_key = create_key(&app, &admin, &["project"]).await;
    mcp_initialize(&app, &proj_key).await;

    // project scope 调 memory 域 → 拒
    let (_, v) = mcp_rpc(
        &app,
        &proj_key,
        rpc(
            2,
            "tools/call",
            json!({"name": "memory", "arguments": {"action": "search", "query": "x"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("memory scope"),
        "project key 调 memory 域应被拒：{v}"
    );

    // memory-only key：tools/list 不见 projects，调用被拒
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let names = tool_names(&app, &mem_key).await;
    assert!(
        !names.contains(&"projects".to_string()),
        "memory-only key 不应看到 projects：{names:?}"
    );
    let v = act_raw(
        &app,
        &mem_key,
        3,
        "create",
        json!({"name": "x", "type": "dev"}),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("project scope"),
        "memory key 调 projects 应被拒：{v}"
    );

    // 双 scope key 两边都可用
    let both = create_key(&app, &admin, &["memory", "project"]).await;
    let names = tool_names(&app, &both).await;
    assert!(names.contains(&"projects".to_string()));
    assert!(names.contains(&"memory".to_string()));

    // 缺定位参数 → 参数错误（不是内部错误）
    let v = act_raw(&app, &both, 4, "get", json!({})).await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("project_id 或 project_name"),
        "缺定位应提示二选一：{v}"
    );

    // 坏 UUID → 参数错误
    let v = act_raw(&app, &both, 5, "get", json!({"project_id": "not-a-uuid"})).await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("UUID"),
        "坏 UUID 应报参数错误：{v}"
    );
}

/// 管理台同源：build_info 应含 projects 域；action 级停用开关对 projects 同样生效。
#[tokio::test]
async fn project_tools_admin_info_and_toggle() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["project"]).await;

    // GET /settings/mcp：projects 域工具 + 15 操作
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
    let project_tools: Vec<&Value> = tools.iter().filter(|t| t["domain"] == "projects").collect();
    assert_eq!(project_tools.len(), 1, "projects 域应为 1 个域工具");
    assert_eq!(
        project_tools[0]["actions"].as_array().unwrap().len(),
        23,
        "projects 域应展示 23 个操作（含 doc_patch + file 四动作 + link/unlink/links 三动作）：{tools:?}"
    );

    // 停用 projects.delete：目录隐身 + call 拒绝
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/settings/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::from(r#"{"disabled_tools":["projects.delete"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let names = tool_names(&app, &key).await;
    let mut sorted_names = names.clone();
    sorted_names.sort();
    assert_eq!(
        sorted_names,
        vec!["jobs", "projects", "search_all"],
        "域工具应保留（+跨域 search_all；jobs 无域 scope 恒可见）"
    );
    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let description = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "projects")
        .expect("projects 工具应在列")["description"]
        .as_str()
        .unwrap();
    assert!(
        !description.contains("- delete："),
        "停用操作应从目录隐身：{description}"
    );
    assert!(
        description.contains("- create："),
        "其余操作不受影响：{description}"
    );

    let v = act_raw(
        &app,
        &key,
        2,
        "delete",
        json!({"project_id": "00000000-0000-0000-0000-000000000000"}),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已停用"),
        "停用操作调用应报错：{v}"
    );

    // 未知键 400（校验名单覆盖域.action 与工具名）
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
    let description = {
        let (_, v) = mcp_rpc(&app, &key, rpc(3, "tools/list", json!({}))).await;
        expect_result(&v, "tools/list")["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "projects")
            .expect("projects 工具应在列")["description"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(description.contains("- delete："), "恢复后操作应回归目录");
}

/// 未传分类必填字段等 schema 缺参：JSON-RPC 参数错误而非 panic。
#[tokio::test]
async fn project_doc_add_missing_fields_rejected() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    // 缺 category（action 参数必填）→ JSON-RPC 参数错误，报错带 help 提示（L2 自愈）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "doc_add",
            json!({"project_name": "不存在", "title": "t", "content": "c"}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("missing field") && msg.contains("category"),
        "应提示缺字段：{v}"
    );
    assert!(msg.contains("help"), "报错应提示 help（L2 自愈）：{v}");
}

/// 精确寻址读设计：索引模式 → 搜索定位行号 → 区间精读；全文模式无损；
/// 行号边界校验；分类过滤。全程无截断。
#[tokio::test]
async fn project_precise_addressing_read() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let p = act_json(
        &app,
        &key,
        "create",
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
    let doc = act_json(
        &app,
        &key,
        "doc_add",
        json!({"project_id": pid, "category": "后端", "title": "进度", "content": progress}),
    )
    .await;
    let doc_id = doc["id"].as_str().unwrap().to_string();
    let other = act_json(
        &app,
        &key,
        "doc_add",
        json!({"project_id": pid, "category": "规划", "title": "结论", "content": "结论一：MCP 并入 /mcp\n结论二：scope 分权"}),
    )
    .await;

    // 索引模式：无正文，有 content_chars
    let idx = act_json(&app, &key, "get", json!({"project_id": pid})).await;
    let docs = idx["docs"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    for d in docs {
        assert!(d.get("content").is_none(), "索引模式不带正文：{d}");
        assert!(d["content_chars"].as_i64().unwrap() > 0, "{d}");
        assert!(d.get("id").is_some() && d.get("title").is_some(), "{d}");
    }

    // 全量模式：无损拿回原文
    let full = act_json(
        &app,
        &key,
        "get",
        json!({"project_id": pid, "include_content": true}),
    )
    .await;
    assert_eq!(full["docs"].as_array().unwrap().len(), 2);
    assert_eq!(full["docs"][0]["content"], json!(progress));

    // 分类过滤
    let filtered = act_json(
        &app,
        &key,
        "get",
        json!({"project_id": pid, "category": "规划"}),
    )
    .await;
    let fd = filtered["docs"].as_array().unwrap();
    assert_eq!(fd.len(), 1);
    assert_eq!(fd[0]["title"], "结论");

    // 搜索定位：命中行号 + 原文行（大小写不敏感）
    let hits = act_json(
        &app,
        &key,
        "doc_search",
        json!({"project_id": pid, "query": "Streamable HTTP"}),
    )
    .await;
    let arr = hits.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{hits}");
    assert_eq!(arr[0]["doc_id"], json!(doc_id));
    assert_eq!(arr[0]["line"], 7);
    assert_eq!(arr[0]["text"], "第七行提到 streamable http 传输");

    // 搜索跨行：同一篇文档多行命中
    let multi = act_json(
        &app,
        &key,
        "doc_search",
        json!({"project_name": "寻址读", "query": "结论"}),
    )
    .await;
    let mh = multi.as_array().unwrap();
    assert_eq!(mh.len(), 2, "{multi}");
    assert_eq!(mh[0]["line"], 1);
    assert_eq!(mh[1]["line"], 2);

    // 区间精读 5-9 行：恒带行号前缀，端点含入
    let range = act_json(
        &app,
        &key,
        "doc_get",
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
    let tail = act_json(
        &app,
        &key,
        "doc_get",
        json!({"doc_id": doc_id, "start_line": 11}),
    )
    .await;
    assert_eq!(tail["end_line"], 12);
    assert!(tail["content"].as_str().unwrap().contains("12: 第12行"));

    // 全文模式：无损原文 + total_lines；with_line_numbers 加行号
    let whole = act_json(&app, &key, "doc_get", json!({"doc_id": doc_id})).await;
    assert_eq!(whole["content"], json!(progress));
    assert_eq!(whole["total_lines"], 12);
    let numbered = act_json(
        &app,
        &key,
        "doc_get",
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
        let (_, v) = mcp_rpc(&app, &key, call(20, "doc_get", bad)).await;
        assert!(v.get("error").is_some(), "坏区间应报错：{v}");
    }
    // 空检索词
    let v = act_raw(
        &app,
        &key,
        21,
        "doc_search",
        json!({"project_id": pid, "query": "  "}),
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
    let limited = act_json(
        &app,
        &key,
        "doc_search",
        json!({"project_id": pid, "query": "行", "limit": 2}),
    )
    .await;
    assert_eq!(limited.as_array().unwrap().len(), 2);

    // 覆盖 other 引用，避免未使用告警
    assert_eq!(other["title"], "结论");
}

/// 项目文件五动作端到端：file_put（新建 v1）→ file_get → 覆盖（v2 + 快照）→
/// file_get 读历史版本 → file_list → file_delete。
#[tokio::test]
async fn project_file_lifecycle_mcp_end_to_end() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["project"]).await;
    mcp_initialize(&app, &key).await;

    let created = act_json(
        &app,
        &key,
        "create",
        json!({"name": "项目文件 MCP 验证", "type": "dev"}),
    )
    .await;
    let pid = created["id"].as_str().unwrap().to_string();

    // ① file_put 新建 → v1，扩展名推断 mime
    let up = act_json(
        &app,
        &key,
        "file_put",
        json!({"project_id": pid, "name": "architecture.html", "content": "<h1>v1</h1>"}),
    )
    .await;
    assert_eq!(up["version"], 1, "新建应为 v1：{up}");
    assert_eq!(up["mime"], "text/html", "扩展名应推断 mime：{up}");

    // ② file_get 读当前
    let got = act_json(
        &app,
        &key,
        "file_get",
        json!({"project_id": pid, "name": "architecture.html"}),
    )
    .await;
    assert_eq!(got["content"], "<h1>v1</h1>");
    assert_eq!(got["version"], 1);

    // ③ 覆盖 → v2
    let up2 = act_json(
        &app,
        &key,
        "file_put",
        json!({"project_id": pid, "name": "architecture.html", "content": "<h1>v2</h1>"}),
    )
    .await;
    assert_eq!(up2["version"], 2, "覆盖应 version+1：{up2}");

    // ④ file_get 读历史版本 v1
    let old = act_json(
        &app,
        &key,
        "file_get",
        json!({"project_id": pid, "name": "architecture.html", "version": 1}),
    )
    .await;
    assert_eq!(old["content"], "<h1>v1</h1>", "历史版本应可回读：{old}");
    assert_eq!(old["version"], 1);

    // ⑤ file_list（EN-68②：bytes = UTF-8 字节数与 content_chars 并列）
    let list = act_json(&app, &key, "file_list", json!({"project_id": pid})).await;
    assert_eq!(list.as_array().unwrap().len(), 1, "应只有 1 个文件：{list}");
    assert_eq!(list[0]["name"], "architecture.html");
    assert_eq!(
        list[0]["bytes"], 11,
        "bytes 应为 UTF-8 字节数（<h1>v2</h1> = 11 字节）：{list}"
    );
    assert_eq!(list[0]["content_chars"], 11);
    let cur = act_json(
        &app,
        &key,
        "file_get",
        json!({"project_id": pid, "name": "architecture.html"}),
    )
    .await;
    assert_eq!(cur["bytes"], 11, "file_get 应带 bytes：{cur}");
    assert_eq!(cur["content"], "<h1>v2</h1>");

    // ⑥ 非法文件名拒绝（路径分隔）
    let v = act_raw(
        &app,
        &key,
        30,
        "file_put",
        json!({"project_id": pid, "name": "a/b.html", "content": "x"}),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("路径分隔符"), "应拒绝路径分隔符：{v}");

    // ⑦ file_delete
    let del = act_json(
        &app,
        &key,
        "file_delete",
        json!({"project_id": pid, "name": "architecture.html"}),
    )
    .await;
    assert_eq!(del["deleted"], "architecture.html");
    let v2 = act_raw(
        &app,
        &key,
        31,
        "file_get",
        json!({"project_id": pid, "name": "architecture.html"}),
    )
    .await;
    assert!(
        v2["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不存在"),
        "删除后应 404：{v2}"
    );
}
