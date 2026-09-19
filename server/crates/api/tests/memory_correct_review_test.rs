//! correct 快路径 + 待审 AI 复核（confirm/discard）集成测试（收录哲学线 task-2）。
//!
//! 治理边界：correct 仅 active 非敏感原子（单事务取代链：旧 superseded + superseded_by 指针）；
//! confirm/discard 仅 needs_review=true 条目——正常记忆对 AI 只读（AI 代管复核的授权边界）。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, login_token, mcp_call_json, mcp_rpc, rpc};
use tower::util::ServiceExt;

struct Ctx {
    app: axum::Router,
    token: String,
    /// 测试库守卫：随 Ctx 活到测试结束（提前 drop 会中途 FORCE 删库）
    _pg: support::TestPg,
}

impl Ctx {
    async fn new() -> Self {
        let (app, _pg) = app().await;
        let token = login_token(&app).await;
        Self { app, token, _pg }
    }

    async fn req(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json");
        let req = match body {
            Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, v)
    }

    /// MCP memory 域调用（成功路径）
    async fn mem(&self, action: &str, args: Value) -> Value {
        let mut body = json!({ "action": action });
        if let (Some(dst), Some(src)) = (body.as_object_mut(), args.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        mcp_call_json(&self.app, &self.token, "memory", body).await
    }

    /// MCP memory 域调用（负路径——返回 JSON-RPC error message）
    async fn mem_err(&self, action: &str, args: Value) -> String {
        let mut body = json!({ "action": action });
        if let (Some(dst), Some(src)) = (body.as_object_mut(), args.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        let (_, v) = mcp_rpc(
            &self.app,
            &self.token,
            rpc(
                2,
                "tools/call",
                json!({ "name": "memory", "arguments": body }),
            ),
        )
        .await;
        v["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("{action} 应返回 error：{v}"))
            .to_string()
    }

    async fn remember_fact(&self, text: &str) -> String {
        let v = self
            .mem("remember", json!({ "text": text, "strength": "fact" }))
            .await;
        v["id"]
            .as_str()
            .unwrap_or_else(|| panic!("remember 应返回原子 id：{v}"))
            .to_string()
    }

    async fn atom_by_id(&self, id: &str) -> Value {
        // 单原子 GET 不存在（/memory/atoms/{id} 仅 PATCH）——走列表 + id 过滤
        let (status, v) = self.req("GET", "/memory/atoms?limit=500", None).await;
        assert_eq!(status, StatusCode::OK, "查原子列表失败：{v}");
        v.as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|a| a["id"] == Value::String(id.to_string()))
            })
            .cloned()
            .unwrap_or_else(|| panic!("原子 {id} 不在列表中：{v}"))
    }

    async fn mark_review(&self, id: &str) {
        let (status, v) = self
            .req(
                "PATCH",
                &format!("/memory/atoms/{id}"),
                Some(json!({ "needs_review": true })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "标记待审失败：{v}");
    }
}

#[tokio::test]
async fn correct_supersede_chain_and_guards() {
    let ctx = Ctx::new().await;

    // 1. remember fact 建目标原子
    let old_id = ctx.remember_fact("用户住在上海").await;

    // 2. correct：单事务取代链——新原子 active
    let v = ctx
        .mem(
            "correct",
            json!({ "target_id": old_id, "text": "用户现居杭州" }),
        )
        .await;
    let new_id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["status"], "active");
    assert_eq!(v["content"], "用户现居杭州");
    assert_ne!(new_id, old_id);

    // 3. 旧原子 superseded + superseded_by 指向新原子
    let old = ctx.atom_by_id(&old_id).await;
    assert_eq!(old["status"], "superseded");
    assert_eq!(old["superseded_by"], Value::String(new_id.clone()));

    // 4. 治理：对已取代原子再 correct → 报错（non-active）
    let err = ctx
        .mem_err(
            "correct",
            json!({ "target_id": old_id, "text": "用户搬去苏州" }),
        )
        .await;
    assert!(err.contains("active"), "应报非 active：{err}");

    // 5. 治理：不存在的 target → 报错
    let err = ctx
        .mem_err(
            "correct",
            json!({
                "target_id": "00000000-0000-4000-8000-000000000099",
                "text": "任意"
            }),
        )
        .await;
    assert!(err.contains("不存在"), "应报不存在：{err}");
}

#[tokio::test]
async fn review_confirm_discard_guards() {
    let ctx = Ctx::new().await;

    let a = ctx.remember_fact("待审条目甲").await;
    let b = ctx.remember_fact("正常条目乙").await;
    let c = ctx.remember_fact("待审条目丙").await;

    // 标记 A/C 进待审
    ctx.mark_review(&a).await;
    ctx.mark_review(&c).await;

    // confirm A：摘标记
    let v = ctx.mem("confirm", json!({ "atom_id": a })).await;
    assert_eq!(v["needs_review"], false);
    let a_row = ctx.atom_by_id(&a).await;
    assert_eq!(a_row["needs_review"], false);

    // 治理：confirm 非待审条目 → 报错
    let err = ctx.mem_err("confirm", json!({ "atom_id": b })).await;
    assert!(err.contains("needs_review"), "应报非待审：{err}");

    // discard C：归档
    let v = ctx.mem("discard", json!({ "atom_id": c })).await;
    assert_eq!(v["status"], "archived");
    let c_row = ctx.atom_by_id(&c).await;
    assert_eq!(c_row["status"], "archived");

    // 治理：discard 不存在 id → 报错（找不到或非待审）
    let err = ctx
        .mem_err(
            "discard",
            json!({ "atom_id": "00000000-0000-4000-8000-000000000099" }),
        )
        .await;
    assert!(err.contains("needs_review"), "应报非待审/不存在：{err}");

    let _ = b; // b 作为对照组，全程未被处置
}

#[tokio::test]
async fn correct_sensitive_guard() {
    let ctx = Ctx::new().await;

    let d = ctx.remember_fact("用户的过敏史记录条目").await;
    let (status, v) = ctx
        .req(
            "PATCH",
            &format!("/memory/atoms/{d}"),
            Some(json!({ "sensitive": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "标记敏感失败：{v}");

    let err = ctx
        .mem_err("correct", json!({ "target_id": d, "text": "用户无过敏史" }))
        .await;
    assert!(err.contains("敏感"), "应报敏感禁碰：{err}");
}

#[tokio::test]
async fn persona_edit_pins_and_guards() {
    let ctx = Ctx::new().await;

    // 1. persona_edit 落库：version+1 + manually_edited=true（钉住）
    let v = ctx
        .mem(
            "persona_edit",
            json!({ "aspect": "skills", "content": "用户在学 Rust，偏好实战项目驱动" }),
        )
        .await;
    assert_eq!(v["aspect"], "skills");
    assert_eq!(v["manually_edited"], true);
    let v1 = v["version"].as_i64().unwrap();

    // 2. 再编辑一次：version 递增
    let v = ctx
        .mem(
            "persona_edit",
            json!({ "aspect": "skills", "content": "用户在学 Rust 与 Elixir，偏好实战项目驱动" }),
        )
        .await;
    assert_eq!(v["version"].as_i64().unwrap(), v1 + 1);
    assert_eq!(v["manually_edited"], true);

    // 3. 治理：非法 aspect → 可行动报错
    let err = ctx
        .mem_err("persona_edit", json!({ "aspect": "mood", "content": "x" }))
        .await;
    assert!(err.contains("aspect"), "应报 aspect 非法：{err}");
}

#[tokio::test]
async fn distill_trigger_guard_and_full() {
    let ctx = Ctx::new().await;

    // 1. mode=sleep 预留：报未上线
    let err = ctx.mem_err("distill", json!({ "mode": "sleep" })).await;
    assert!(err.contains("尚未上线"), "应报 sleep 未上线：{err}");

    // 2. 无 running：触发成功（full 含 consolidate）
    let v = ctx.mem("distill", json!({ "full": true })).await;
    assert_eq!(v["already_running"], false);
    let kinds: Vec<&str> = v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"extract_atoms"), "应含 extract_atoms：{v}");
    assert!(kinds.contains(&"consolidate"), "full 应含 consolidate：{v}");

    // 3. 撞车守卫：手工造一条 running extract_atoms（直连测试库）→
    //    already_running 且无新 job 入队（任务列表不留空跑记录）
    let url = support::connection_url(&ctx._pg).await.unwrap();
    let pool = sqlx::postgres::PgPool::connect(&url).await.unwrap();
    sqlx::query(
        "INSERT INTO jobs (id, kind, payload, status, attempts, max_attempts) \
         VALUES ('00000000-0000-4000-8000-aaaaaaaaaaaa', 'extract_atoms', '{}', 'running', 0, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'extract_atoms'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let v = ctx.mem("distill", json!({})).await;
    assert_eq!(v["already_running"], true);

    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'extract_atoms'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, before, "撞车时不应有新 job 入队");
}

#[tokio::test]
async fn original_scope_guards_kv_access() {
    let ctx = Ctx::new().await;
    let admin = ctx.token.clone();

    // 签三把 key：memory-only / original / original:ro（动作级只读）
    let mem_key = support::create_key(&ctx.app, &admin, &["memory"]).await;
    let orig_key = support::create_key(&ctx.app, &admin, &["original"]).await;
    let orig_ro = support::create_key(&ctx.app, &admin, &["original:ro"]).await;

    // 1. original key：kv_put → kv_get 全链可用
    let put = mcp_call_json(
        &ctx.app,
        &orig_key,
        "memory",
        json!({
            "action": "kv_put",
            "key": "test/original-scope",
            "value": "secret-value-123",
            "context": "收录哲学线集成测试",
        }),
    )
    .await;
    assert_eq!(put["value"], "secret-value-123");

    let get = mcp_call_json(
        &ctx.app,
        &orig_key,
        "memory",
        json!({ "action": "kv_get", "key": "test/original-scope" }),
    )
    .await;
    assert_eq!(get["value"], "secret-value-123");

    // 2. 治理：memory-only key → kv_get 被拒（error 含 original——凭据不再搭 memory 的车）
    let (_, v) = mcp_rpc(
        &ctx.app,
        &mem_key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "memory",
                "arguments": { "action": "kv_get", "key": "test/original-scope" }
            }),
        ),
    )
    .await;
    let msg = v["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("memory-only key 的 kv_get 应被拒：{v}"));
    assert!(msg.contains("original"), "应报缺 original scope：{msg}");

    // 3. original:ro key：kv_get 可用（读），kv_put 被拒（写——动作级权限）
    let get = mcp_call_json(
        &ctx.app,
        &orig_ro,
        "memory",
        json!({ "action": "kv_get", "key": "test/original-scope" }),
    )
    .await;
    assert_eq!(get["value"], "secret-value-123");

    let (_, v) = mcp_rpc(
        &ctx.app,
        &orig_ro,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "memory",
                "arguments": { "action": "kv_put", "key": "test/ro-write", "value": "x" }
            }),
        ),
    )
    .await;
    let msg = v["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!(":ro key 的 kv_put 应被拒：{v}"));
    assert!(msg.contains(":ro"), "应报只读变体拒绝：{msg}");
}
