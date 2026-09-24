//! 路由注册与中间件装配。

pub mod assets_api;
pub mod auth_api;
pub mod codegraph_api;
pub mod health;
pub mod jobs_api;
pub mod llm_api;
pub mod memory_api;
pub mod migrate_api;
pub mod project_api;
pub mod search_api;
pub mod skills_api;
pub mod todos_api;
pub mod wiki_api;
pub mod wiki_docs_api;

use crate::state::AppState;

mod memory_groups;
use axum::middleware::{Next, from_fn_with_state};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use memory_groups::*;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "engram API", version = env!("CARGO_PKG_VERSION"),
        description = "单用户 AI 长期记忆平台。平台即工具：AI 通过本 API 操纵记忆。"),
    paths(
        health::health, health::ready,
        auth_api::login_handler, auth_api::status, auth_api::init_account,
        auth_api::change_credentials, auth_api::list_sessions, auth_api::revoke_session,
        auth_api::revoke_others, auth_api::username, auth_api::logout,
        jobs_api::list_jobs, jobs_api::get_job, jobs_api::get_job_events, jobs_api::revive_job,
        llm_api::create_provider, llm_api::list_providers, llm_api::test_provider,
        llm_api::update_provider, llm_api::delete_provider, llm_api::reencrypt_providers,
        llm_api::get_routing, llm_api::put_routing, llm_api::suggest_routing, llm_api::usage,
        llm_api::create_api_key_handler, llm_api::list_api_keys, llm_api::revoke_api_key,
        llm_api::batch_revoke_api_keys, llm_api::update_api_key, llm_api::fetch_models,
        crate::mcp_admin::settings_mcp, crate::mcp_admin::settings_mcp_update,
 crate::rhythm_admin::get_rhythm_config, crate::rhythm_admin::put_rhythm_config,
        memory_api::write_session, memory_api::list_sessions, memory_api::get_session,
        memory_api::erase_session, memory_api::append_session, memory_api::import_session, memory_api::void_session, memory_api::restore_session, memory_api::batch_restore_sessions, memory_api::batch_erase_sessions, memory_api::trigger_distill, memory_api::purge_agent, memory_api::export_memory,
        memory_api::list_atoms, memory_api::create_atom, memory_api::update_atom,
        memory_api::list_scenarios, memory_api::get_scenario,
        memory_api::get_persona, memory_api::persona_edit, memory_api::persona_history,
        memory_api::persona_rollback, memory_api::atom_revisions,
        memory_api::search, memory_api::context, memory_api::embedding_status, memory_api::reembed_memory,
        memory_api::list_kv, memory_api::get_kv,
        memory_api::timeline,
        memory_api::list_entities, memory_api::entity_graph, memory_api::search_entities_handler, memory_api::create_entity,
        memory_api::batch_entities, memory_api::export_entities,
        memory_api::get_entity, memory_api::update_entity, memory_api::delete_entity,
        memory_api::attach_atom, memory_api::detach_atom, memory_api::merge_entity, memory_api::entity_revisions,
        memory_api::list_entity_relations, memory_api::create_entity_relation, memory_api::delete_entity_relation,
        search_api::search,
        wiki_docs_api::submit_url, wiki_docs_api::upload, wiki_docs_api::list_documents,
        wiki_docs_api::get_document, wiki_docs_api::document_chunks,
        wiki_docs_api::delete_document, wiki_docs_api::reembed, wiki_docs_api::search,
        wiki_api::ingest, wiki_api::list_pages, wiki_api::get_page, wiki_api::put_page,
        wiki_api::graph, wiki_api::lint, wiki_api::apply_proposal, wiki_api::search,
        wiki_api::query_gaps,
        wiki_api::rebuild_links,
        wiki_api::duplicates,
        wiki_api::list_proposals,
        wiki_api::get_purpose, wiki_api::set_purpose,
        wiki_api::list_reviews, wiki_api::resolve_review,
        wiki_api::repair,
        wiki_api::repair_async,
        wiki_api::merge_pages,
        wiki_api::archive_query, wiki_api::list_sources, wiki_api::delete_source,
        wiki_api::promote, wiki_api::promotions, wiki_api::rebuild_tsv,
        wiki_api::insights, wiki_api::dismiss_insight, wiki_api::reset_insights,
        codegraph_api::register_project, codegraph_api::upload_artifact,
        codegraph_api::list_projects,
        codegraph_api::get_project, codegraph_api::delete_project,
        codegraph_api::index_project, codegraph_api::sync_project,
        codegraph_api::query, codegraph_api::status, codegraph_api::graph,
        codegraph_api::gc, codegraph_api::project_artifact,
        migrate_api::export_bundle, migrate_api::import_bundle, migrate_api::pull,
    migrate_api::migrate_sync,
        todos_api::list_todos, todos_api::create_todo, todos_api::get_todo,
        todos_api::update_todo, todos_api::delete_todo, todos_api::todo_links, todos_api::export_todos,
        project_api::list_types, project_api::create_project, project_api::list_projects,
        project_api::graph,
        project_api::get_project, project_api::update_project, project_api::delete_project,
        project_api::batch_delete_projects,
        project_api::add_location, project_api::get_location, project_api::update_location, project_api::delete_location,
        project_api::add_link, project_api::list_links, project_api::delete_link,
        project_api::add_doc, project_api::get_doc, project_api::update_doc, project_api::delete_doc,
        project_api::upsert_file, project_api::list_files, project_api::get_file, project_api::delete_file,
        project_api::list_file_versions, project_api::get_file_version,
        assets_api::list_kinds, assets_api::list_assets, assets_api::create_asset,
        assets_api::get_asset, assets_api::update_asset, assets_api::delete_asset,
        skills_api::list_skills, skills_api::create_skill, skills_api::import_skills,
        skills_api::export_skills, skills_api::get_skill, skills_api::update_skill,
        skills_api::delete_skill, skills_api::list_revisions, skills_api::restore_revision,
        skills_api::import_transfer,
        skills_api::list_files, skills_api::get_file, skills_api::put_file, skills_api::delete_file,
    ),
)]
pub(crate) struct ApiDoc;

pub fn router(state: AppState) -> Router {
    let public = public_routes(&state);
    let authed = Router::new()
        .merge(mcp_routes(state.clone()))
        .merge(account_routes())
        .merge(jobs_routes())
        .merge(settings_routes())
        .merge(llm_routes())
        .merge(memory_routes())
        .merge(search_routes())
        .merge(wiki_routes())
        .merge(codegraph_routes())
        .merge(projects_routes())
        .merge(assets_routes())
        .merge(skills_routes())
        .merge(todos_routes())
        .merge(migrate_routes());

    Router::new()
        .merge(public)
        .merge(authed.layer(from_fn_with_state(state.clone(), crate::auth::bearer_auth)))
        // 产物上传（P001-4）：codegraph db 经 MCP JSON-RPC base64 直传——
        // 默认 2MB 不够（db 上限 256MB × base64 膨胀 1.33 ≈ 349MB body）。
        // 注意：MCP 大 body **真正生效的那道在 rmcp 里**（StreamableHttpService 的
        // max_request_body_bytes，默认 4MB）——此处与 engram_mcp::MAX_REQUEST_BODY_BYTES
        // 同口径（单一事实源）；2026-09-21 活体实测抓出两者不同步会让 push 全挂。
        .layer(axum::extract::DefaultBodyLimit::max(
            engram_mcp::MAX_REQUEST_BODY_BYTES,
        ))
        // 客户端 IP 注入（活跃会话归因）：XFF 首段优先，直连取对端地址（main 以 connect_info 启动）
        .layer(axum::middleware::from_fn(
            crate::client_ip::inject_client_ip,
        ))
        // R10：HTTP 指标（请求计数 + 延迟直方图，按路由模板聚合）——放最外层，覆盖全部 API 路由
        .layer(axum::middleware::from_fn(http_metrics_mw))
        // SPA 静态资源兜底（API 路由未命中时 → web/dist）
        .fallback_service(axum::routing::any(crate::web_assets::static_handler))
        .with_state(state)
}

/// 无鉴权段：健康检查 / 指标 / OpenAPI / 登录与初始化。
fn public_routes(state: &AppState) -> Router<AppState> {
    let metrics_handle = crate::metrics::install();
    Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/metrics", get(crate::metrics::metrics_handler))
        .layer(axum::Extension(metrics_handle))
        .layer(axum::Extension(state.pool.clone()))
        .route("/openapi.json", get(openapi_json))
        // /auth/username、/auth/account、/auth/sessions* 需 Principal（Bearer 注入）——
        // 挂 authed 段（RJ-01 深层修复：误挂 public 时 Extension 提取失败整组 500）
        .route("/auth/login", post(auth_api::login_handler))
        .route("/auth/status", get(auth_api::status))
        .route("/auth/init", post(auth_api::init_account))
}

/// MCP 工具面（nest 进 authed：复用 Bearer；gate 在 Bearer 内、MCP 前）。
fn mcp_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // MCP（用户记忆域工具面）：nest 在 authed 内 → 复用 Bearer 中间件，
        // 每个 JSON-RPC 请求独立认证（key 吊销即刻生效，会话保活不能豁免）；
        // gate 在 Bearer 之内、MCP 之前——服务总开关关闭时对已认证客户端也 503
        .merge(
            Router::new()
                .nest_service("/mcp", engram_mcp::service(state.clone()))
                .route_layer(from_fn_with_state(state.clone(), engram_mcp::gate)),
        )
}

/// `/account` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn account_routes() -> Router<AppState> {
    Router::new()
        // 账号面端点（RJ-01 深层修复 2026-09-18）：需 Principal 的会话/账号操作挂 authed
        // —— 此前误挂 public 段（无 Bearer 注入 Principal），Extension 提取失败整组 500：
        // 「吊销其他设备」「会话列表」「改密码」「用户名」在生产全部不可用。
        // revoke-others 路径同步对齐文档口径（前端/OpenAPI 注解/报错文案三处一致）。
        .route("/auth/username", get(auth_api::username))
        .route(
            "/auth/account",
            axum::routing::put(auth_api::change_credentials),
        )
        .route("/auth/logout", post(auth_api::logout))
        .route("/auth/sessions", get(auth_api::list_sessions))
        .route(
            "/auth/sessions/revoke-others",
            post(auth_api::revoke_others),
        )
        .route(
            "/auth/sessions/{id}",
            axum::routing::delete(auth_api::revoke_session),
        )
}

/// `/jobs` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn jobs_routes() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(jobs_api::list_jobs))
        .route("/jobs/{id}", get(jobs_api::get_job))
        .route("/jobs/{id}/events", get(jobs_api::get_job_events))
        .route("/jobs/{id}/revive", post(jobs_api::revive_job))
}

/// `/settings` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn settings_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/settings/llm/providers",
            post(llm_api::create_provider).get(llm_api::list_providers),
        )
        .route(
            "/settings/llm/providers/models",
            post(llm_api::fetch_models),
        )
        .route(
            "/settings/llm/providers/{id}",
            put(llm_api::update_provider).delete(llm_api::delete_provider),
        )
        .route(
            "/settings/llm/providers/re-encrypt",
            post(llm_api::reencrypt_providers),
        )
        .route(
            "/settings/llm/providers/{id}/test",
            post(llm_api::test_provider),
        )
        .route(
            "/settings/llm/routing",
            get(llm_api::get_routing).put(llm_api::put_routing),
        )
        .route(
            "/settings/llm/routing/suggest",
            post(llm_api::suggest_routing),
        )
        .route(
            "/settings/api-keys",
            post(llm_api::create_api_key_handler).get(llm_api::list_api_keys),
        )
        .route(
            "/settings/api-keys/{id}",
            axum::routing::put(llm_api::update_api_key),
        )
        .route(
            "/settings/api-keys/{id}/revoke",
            post(llm_api::revoke_api_key),
        )
        .route(
            "/settings/api-keys/batch-revoke",
            post(llm_api::batch_revoke_api_keys),
        )
        .route(
            "/settings/mcp",
            get(crate::mcp_admin::settings_mcp).put(crate::mcp_admin::settings_mcp_update),
        )
        .route(
            "/settings/rhythm",
            get(crate::rhythm_admin::get_rhythm_config).put(crate::rhythm_admin::put_rhythm_config),
        )
}

/// `/llm` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn llm_routes() -> Router<AppState> {
    Router::new().route("/llm/usage", get(llm_api::usage))
}

/// `/memory` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn memory_routes() -> Router<AppState> {
    Router::new()
        .merge(memory_routes_group0())
        .merge(memory_routes_group1())
        .merge(memory_routes_group2())
}

/// `/search` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn search_routes() -> Router<AppState> {
    Router::new().route("/search", post(search_api::search))
}

/// `/wiki` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn wiki_routes() -> Router<AppState> {
    Router::new()
        // 文档知识并入 Wiki 前缀（/wiki/documents、/wiki/upload）
        .route(
            "/wiki/documents",
            post(wiki_docs_api::submit_url).get(wiki_docs_api::list_documents),
        )
        .route("/wiki/upload", post(wiki_docs_api::upload))
        // search 先于 {id}，避免 "search" 被当作 id 解析
        .route("/wiki/documents/search", post(wiki_docs_api::search))
        .route(
            "/wiki/documents/{id}",
            get(wiki_docs_api::get_document).delete(wiki_docs_api::delete_document),
        )
        .route(
            "/wiki/documents/{id}/chunks",
            get(wiki_docs_api::document_chunks),
        )
        .route(
            "/wiki/documents/{id}/re-embed",
            post(wiki_docs_api::reembed),
        )
        .route("/wiki/ingest", post(wiki_api::ingest))
        .route("/wiki/pages", get(wiki_api::list_pages))
        .route("/wiki/folders", get(wiki_api::list_folders))
        .route(
            "/wiki/pages/{slug}",
            get(wiki_api::get_page)
                .put(wiki_api::put_page)
                .delete(wiki_api::delete_page),
        )
        .route("/wiki/pages/merge", post(wiki_api::merge_pages))
        .route("/wiki/graph", get(wiki_api::graph))
        .route("/wiki/duplicates", get(wiki_api::duplicates))
        .route("/wiki/lint", post(wiki_api::lint))
        .route("/wiki/query-gaps", get(wiki_api::query_gaps))
        .route("/wiki/links/rebuild", post(wiki_api::rebuild_links))
        .route("/wiki/tsv/rebuild", post(wiki_api::rebuild_tsv))
        .route("/wiki/promote", post(wiki_api::promote))
        .route("/wiki/promotions", get(wiki_api::promotions))
        .route("/wiki/proposals", get(wiki_api::list_proposals))
        .route("/wiki/proposals/apply", post(wiki_api::apply_proposal))
        .route("/wiki/search", post(wiki_api::search))
        .route(
            "/wiki/purpose",
            get(wiki_api::get_purpose).put(wiki_api::set_purpose),
        )
        .route("/wiki/reviews", get(wiki_api::list_reviews))
        .route("/wiki/reviews/{id}/resolve", post(wiki_api::resolve_review))
        .route("/wiki/repair", post(wiki_api::repair))
        .route("/wiki/repair/async", post(wiki_api::repair_async))
        .route("/wiki/queries/archive", post(wiki_api::archive_query))
        .route("/wiki/sources", get(wiki_api::list_sources))
        .route(
            "/wiki/sources/{id}",
            axum::routing::delete(wiki_api::delete_source),
        )
        .route("/wiki/insights", post(wiki_api::insights))
        .route("/wiki/insights/dismiss", post(wiki_api::dismiss_insight))
        .route("/wiki/insights/reset", post(wiki_api::reset_insights))
}

/// `/codegraph` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn codegraph_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/codegraph/projects",
            post(codegraph_api::register_project).get(codegraph_api::list_projects),
        )
        // 产物上传（2026-09-21 入口收敛）：multipart name+file(+head)——
        // 上限用产物口径（257MB），不用全局那道 MCP base64 的 349MB（对 multipart 过宽）。
        .route(
            "/codegraph/artifacts",
            post(codegraph_api::upload_artifact).layer(axum::extract::DefaultBodyLimit::max(
                codegraph_api::MAX_ARTIFACT_BODY_BYTES,
            )),
        )
        .route("/codegraph/status", get(codegraph_api::status))
        .route("/codegraph/gc", post(codegraph_api::gc))
        .route(
            "/codegraph/projects/{id}",
            get(codegraph_api::get_project).delete(codegraph_api::delete_project),
        )
        .route("/codegraph/projects/{id}/graph", get(codegraph_api::graph))
        .route(
            "/codegraph/projects/{id}/index",
            post(codegraph_api::index_project),
        )
        .route(
            "/codegraph/projects/{id}/sync",
            post(codegraph_api::sync_project),
        )
        .route("/codegraph/projects/{id}/query", post(codegraph_api::query))
        .route(
            "/codegraph/projects/{id}/artifact",
            get(codegraph_api::project_artifact),
        )
}

/// `/projects` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
/// 资产台账域路由（2026-09-21 新增）：`types` 先于 `{id}`，避免被当作 id 解析。
fn assets_routes() -> Router<AppState> {
    Router::new()
        .route("/assets/types", get(assets_api::list_kinds))
        .route(
            "/assets",
            post(assets_api::create_asset).get(assets_api::list_assets),
        )
        .route(
            "/assets/{id}",
            get(assets_api::get_asset)
                .put(assets_api::update_asset)
                .delete(assets_api::delete_asset),
        )
}

fn projects_routes() -> Router<AppState> {
    Router::new()
        // 项目记忆域：types 与 batch-delete 先于 {id}，避免被当作 id 解析
        .route("/projects/types", get(project_api::list_types))
        .route("/projects/graph", get(project_api::graph))
        .route(
            "/projects/batch-delete",
            post(project_api::batch_delete_projects),
        )
        .route(
            "/projects",
            post(project_api::create_project).get(project_api::list_projects),
        )
        .route(
            "/projects/{id}",
            get(project_api::get_project)
                .put(project_api::update_project)
                .delete(project_api::delete_project),
        )
        .route("/projects/{id}/locations", post(project_api::add_location))
        .route(
            "/projects/{id}/locations/{loc_id}",
            get(project_api::get_location)
                .put(project_api::update_location)
                .delete(project_api::delete_location),
        )
        // 项目关联（project_links，0058）：一层有向边（part_of 隶属 / related 相关）
        .route(
            "/projects/{id}/links",
            post(project_api::add_link).get(project_api::list_links),
        )
        .route(
            "/projects/{id}/links/{link_id}",
            delete(project_api::delete_link),
        )
        .route(
            "/projects/{id}/files",
            put(project_api::upsert_file).get(project_api::list_files),
        )
        .route(
            "/projects/{id}/files/{name}",
            get(project_api::get_file).delete(project_api::delete_file),
        )
        .route(
            "/projects/{id}/files/{name}/versions",
            get(project_api::list_file_versions),
        )
        .route(
            "/projects/{id}/files/{name}/versions/{version}",
            get(project_api::get_file_version),
        )
        .route("/projects/{id}/docs", post(project_api::add_doc))
        .route(
            "/projects/{id}/docs/{doc_id}",
            get(project_api::get_doc)
                .put(project_api::update_doc)
                .delete(project_api::delete_doc),
        )
}

/// `/skills` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn skills_routes() -> Router<AppState> {
    Router::new()
        // 技能域：import/export 先于 {slug}，避免被当作 slug 解析
        .route("/skills/import", post(skills_api::import_skills))
        .route("/skills/{slug}/files", get(skills_api::list_files))
        .route(
            "/skills/{slug}/file",
            get(skills_api::get_file)
                .put(skills_api::put_file)
                .delete(skills_api::delete_file),
        )
        .route("/skills/export", get(skills_api::export_skills))
        .route("/skills/import-transfer", post(skills_api::import_transfer))
        .route(
            "/skills",
            post(skills_api::create_skill).get(skills_api::list_skills),
        )
        .route(
            "/skills/{slug}",
            get(skills_api::get_skill)
                .put(skills_api::update_skill)
                .delete(skills_api::delete_skill),
        )
        .route("/skills/{slug}/revisions", get(skills_api::list_revisions))
        .route(
            "/skills/{slug}/revisions/{rev_id}/restore",
            post(skills_api::restore_revision),
        )
}

/// `/todos` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn todos_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/todos",
            get(todos_api::list_todos).post(todos_api::create_todo),
        )
        .route(
            "/todos/{id}",
            get(todos_api::get_todo)
                .put(todos_api::update_todo)
                .delete(todos_api::delete_todo),
        )
        .route("/todos/{id}/links", get(todos_api::todo_links))
        .route("/todos/export", get(todos_api::export_todos))
}

/// `/migrate` 域路由组（自 `router()` 按域拆出，纯搬移，零行为变化）。
fn migrate_routes() -> Router<AppState> {
    Router::new()
        .route("/migrate/export", get(migrate_api::export_bundle))
        .route("/migrate/import", post(migrate_api::import_bundle))
        .route("/migrate/pull", post(migrate_api::pull))
        .route("/migrate/sync", post(migrate_api::migrate_sync))
}

/// R10：HTTP 指标中间件——请求计数 + 延迟直方图，route 用匹配模板（非逐 URI，防高基数）。
async fn http_metrics_mw(req: axum::extract::Request, next: Next) -> axum::response::Response {
    let start = std::time::Instant::now();
    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());

    let resp = next.run(req).await;

    let status = resp.status().as_u16().to_string();
    metrics::counter!(
        "http_requests_total",
        "method" => method.to_string(),
        "route" => route.clone(),
        "status" => status
    )
    .increment(1);
    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method.to_string(),
        "route" => route
    )
    .record(start.elapsed().as_secs_f64());
    resp
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// OpenAPI 文档（程序化访问，openapi-dump bin 用）。
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
