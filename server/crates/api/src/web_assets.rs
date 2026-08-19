//! 前端静态资源嵌入（单端口交付，Phase 6）。
//! 构建时嵌入 web/dist（Dockerfile 先 build 前端再 COPY 进来；本地开发用 Vite proxy）。

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../../web/dist"]
struct WebAssets;

/// 静态资源 + SPA fallback（未知路径 → index.html）。
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(asset) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                asset.data,
            )
                .into_response()
        }
        // SPA：非 API 路径回退 index.html（React Router 前端路由）
        None => match WebAssets::get("index.html") {
            Some(index) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                index.data,
            )
                .into_response(),
            None => (
                StatusCode::NOT_FOUND,
                "前端资源未构建（web/dist 缺失；开发模式用 Vite dev server）",
            )
                .into_response(),
        },
    }
}
