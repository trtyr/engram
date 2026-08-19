//! 路由注册。

pub mod health;

use crate::state::AppState;
use axum::Json;
use axum::routing::get;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "agent-memory API", version = env!("CARGO_PKG_VERSION"),
        description = "单用户 AI 长期记忆平台。平台即工具：AI 通过本 API 操纵记忆。"),
    paths(health::health, health::ready),
)]
struct ApiDoc;

pub fn router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/openapi.json", get(openapi_json))
        .with_state(state)
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
