//! 路由注册与中间件装配。

pub mod auth_api;
pub mod codegraph_api;
pub mod health;
pub mod jobs_api;
pub mod knowledge_api;
pub mod llm_api;
pub mod memory_api;
pub mod wiki_api;

use crate::state::AppState;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::{Json, Router};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "agent-memory API", version = env!("CARGO_PKG_VERSION"),
        description = "单用户 AI 长期记忆平台。平台即工具：AI 通过本 API 操纵记忆。"),
    paths(
        health::health, health::ready,
        auth_api::login_handler,
        jobs_api::list_jobs, jobs_api::get_job, jobs_api::get_job_events, jobs_api::revive_job,
        llm_api::create_provider, llm_api::list_providers, llm_api::test_provider,
        llm_api::get_routing, llm_api::put_routing, llm_api::usage,
        llm_api::create_api_key_handler, llm_api::list_api_keys, llm_api::revoke_api_key,
        memory_api::write_session, memory_api::list_sessions, memory_api::get_session,
        memory_api::erase_session, memory_api::trigger_distill,
        memory_api::list_atoms, memory_api::create_atom, memory_api::update_atom,
        memory_api::list_scenarios, memory_api::get_scenario,
        memory_api::get_persona, memory_api::persona_history, memory_api::persona_rollback,
        memory_api::search, memory_api::context,
        knowledge_api::submit_url, knowledge_api::upload, knowledge_api::list_documents,
        knowledge_api::get_document, knowledge_api::document_chunks,
        knowledge_api::delete_document, knowledge_api::search,
        wiki_api::ingest, wiki_api::list_pages, wiki_api::get_page, wiki_api::put_page,
        wiki_api::graph, wiki_api::lint, wiki_api::apply_proposal, wiki_api::search,
        wiki_api::get_purpose, wiki_api::set_purpose,
        wiki_api::list_reviews, wiki_api::resolve_review,
        wiki_api::archive_query, wiki_api::list_sources, wiki_api::delete_source,
        wiki_api::insights, wiki_api::dismiss_insight, wiki_api::reset_insights,
        codegraph_api::register_project, codegraph_api::list_projects,
        codegraph_api::get_project, codegraph_api::index_project,
        codegraph_api::sync_project, codegraph_api::query,
    ),
)]
pub(crate) struct ApiDoc;

pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/openapi.json", get(openapi_json))
        .route("/auth/login", post(auth_api::login_handler));

    let authed = Router::new()
        .route("/jobs", get(jobs_api::list_jobs))
        .route("/jobs/{id}", get(jobs_api::get_job))
        .route("/jobs/{id}/events", get(jobs_api::get_job_events))
        .route("/jobs/{id}/revive", post(jobs_api::revive_job))
        .route(
            "/settings/llm/providers",
            post(llm_api::create_provider).get(llm_api::list_providers),
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
            "/settings/api-keys",
            post(llm_api::create_api_key_handler).get(llm_api::list_api_keys),
        )
        .route(
            "/settings/api-keys/{id}/revoke",
            post(llm_api::revoke_api_key),
        )
        .route("/llm/usage", get(llm_api::usage))
        .route(
            "/memory/sessions",
            post(memory_api::write_session).get(memory_api::list_sessions),
        )
        .route(
            "/memory/sessions/{id}",
            get(memory_api::get_session).delete(memory_api::erase_session),
        )
        .route("/memory/distill", post(memory_api::trigger_distill))
        .route(
            "/memory/atoms",
            get(memory_api::list_atoms).post(memory_api::create_atom),
        )
        .route(
            "/memory/atoms/{id}",
            axum::routing::patch(memory_api::update_atom),
        )
        .route("/memory/scenarios", get(memory_api::list_scenarios))
        .route("/memory/scenarios/{id}", get(memory_api::get_scenario))
        .route("/memory/persona", get(memory_api::get_persona))
        .route("/memory/persona/history", get(memory_api::persona_history))
        .route(
            "/memory/persona/rollback",
            post(memory_api::persona_rollback),
        )
        .route("/memory/search", post(memory_api::search))
        .route("/memory/context", get(memory_api::context))
        .route(
            "/knowledge/documents",
            post(knowledge_api::submit_url).get(knowledge_api::list_documents),
        )
        .route("/knowledge/upload", post(knowledge_api::upload))
        .route(
            "/knowledge/documents/{id}",
            get(knowledge_api::get_document).delete(knowledge_api::delete_document),
        )
        .route(
            "/knowledge/documents/{id}/chunks",
            get(knowledge_api::document_chunks),
        )
        .route("/knowledge/search", post(knowledge_api::search))
        .route("/wiki/ingest", post(wiki_api::ingest))
        .route("/wiki/pages", get(wiki_api::list_pages))
        .route(
            "/wiki/pages/{slug}",
            get(wiki_api::get_page).put(wiki_api::put_page),
        )
        .route("/wiki/graph", get(wiki_api::graph))
        .route("/wiki/lint", post(wiki_api::lint))
        .route("/wiki/proposals/apply", post(wiki_api::apply_proposal))
        .route("/wiki/search", post(wiki_api::search))
        .route(
            "/wiki/purpose",
            get(wiki_api::get_purpose).put(wiki_api::set_purpose),
        )
        .route("/wiki/reviews", get(wiki_api::list_reviews))
        .route("/wiki/reviews/{id}/resolve", post(wiki_api::resolve_review))
        .route("/wiki/queries/archive", post(wiki_api::archive_query))
        .route("/wiki/sources", get(wiki_api::list_sources))
        .route(
            "/wiki/sources/{id}",
            axum::routing::delete(wiki_api::delete_source),
        )
        .route("/wiki/insights", post(wiki_api::insights))
        .route("/wiki/insights/dismiss", post(wiki_api::dismiss_insight))
        .route("/wiki/insights/reset", post(wiki_api::reset_insights))
        .route(
            "/codegraph/projects",
            post(codegraph_api::register_project).get(codegraph_api::list_projects),
        )
        .route("/codegraph/projects/{id}", get(codegraph_api::get_project))
        .route(
            "/codegraph/projects/{id}/index",
            post(codegraph_api::index_project),
        )
        .route(
            "/codegraph/projects/{id}/sync",
            post(codegraph_api::sync_project),
        )
        .route("/codegraph/projects/{id}/query", post(codegraph_api::query));

    Router::new()
        .merge(public)
        .merge(authed.layer(from_fn_with_state(state.clone(), crate::auth::bearer_auth)))
        // SPA 静态资源兜底（API 路由未命中时 → web/dist）
        .fallback_service(axum::routing::any(crate::web_assets::static_handler))
        .with_state(state)
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// OpenAPI 文档（程序化访问，openapi-dump bin 用）。
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
