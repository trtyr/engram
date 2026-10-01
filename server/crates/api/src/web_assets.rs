//! 前端静态资源嵌入（单端口交付，Phase 6）。
//! 构建时嵌入 web/dist（Dockerfile 先 build 前端再 COPY 进来；本地开发用 Vite proxy）。
//!
//! 缓存契约（2026-09-27 Lighthouse 修复）：
//! - `/static/*` 文件名带内容 hash → immutable 年缓存（内容变 = 文件名变）
//! - `index.html` 与 SPA fallback → no-cache（每次回源校验，保证拿到新 hash 清单）
//! - `/robots.txt` → 内联纯文本（私有系统全站禁爬，避免 SPA fallback 回 HTML）

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../../web/dist"]
struct WebAssets;

/// 带 hash 的静态资源：内容寻址，可放心缓存一年。
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// 入口 HTML 与 SPA fallback：每次协商校验。
const CACHE_NO_CACHE: &str = "no-cache";
/// robots.txt：私有记忆系统，全站禁爬。
const ROBOTS_TXT: &str = "User-agent: *\nDisallow: /\n";

/// 安全响应头（静态路径在 handler 内直附——Router::layer 不覆盖 fallback_service）。
fn with_security_headers(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(
        "x-frame-options",
        axum::http::HeaderValue::from_static("DENY"),
    );
    h.insert(
        "cross-origin-opener-policy",
        axum::http::HeaderValue::from_static("same-origin"),
    );
    h.insert(
        "content-security-policy",
        axum::http::HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
        ),
    );
    h.insert(
        "strict-transport-security",
        axum::http::HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );
    res
}

/// 静态资源 + SPA fallback（未知路径 → index.html）。
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // robots.txt 优先于 SPA fallback（否则被 fallback 回 HTML，爬虫解析出几十条语法错误）
    if path == "robots.txt" {
        return with_security_headers(
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                ROBOTS_TXT,
            )
                .into_response(),
        );
    }

    match WebAssets::get(path) {
        Some(asset) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if path.starts_with("static/") {
                CACHE_IMMUTABLE
            } else {
                CACHE_NO_CACHE
            };
            with_security_headers(
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, mime.as_ref()),
                        (header::CACHE_CONTROL, cache),
                    ],
                    asset.data,
                )
                    .into_response(),
            )
        }
        // SPA：非 API 路径回退 index.html（React Router 前端路由）
        None => match WebAssets::get("index.html") {
            Some(index) => with_security_headers(
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/html"),
                        (header::CACHE_CONTROL, CACHE_NO_CACHE),
                    ],
                    index.data,
                )
                    .into_response(),
            ),
            None => (
                StatusCode::NOT_FOUND,
                "前端资源未构建（web/dist 缺失；开发模式用 Vite dev server）",
            )
                .into_response(),
        },
    }
}
