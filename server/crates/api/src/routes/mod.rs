//! 路由注册与中间件装配。

pub mod auth_api;
pub mod health;
pub mod jobs_api;
pub mod llm_api;

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
    ),
)]
struct ApiDoc;

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
        .route("/llm/usage", get(llm_api::usage));

    Router::new()
        .merge(public)
        .merge(authed.layer(from_fn_with_state(state.clone(), crate::auth::bearer_auth)))
        .with_state(state)
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
