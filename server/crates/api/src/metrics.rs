//! R10 可观测性：Prometheus /metrics 端点与进程级 recorder。
//!
//! 形态（单用户单机部署）：Prometheus 文本格式挂在 public 路由（抓取器不带 token，
//! 指标只含计数/耗时/队列深度，不含 query 内容与用户数据）；
//! `AGENT_MEMORY_METRICS=0` 可整体关闭端点与记录。
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use engram_storage::PgPool;

/// 安装全局 Prometheus recorder（进程内一次）。返回抓取 handle。
/// env AGENT_MEMORY_METRICS=0 → 返回 None（端点 404、记录宏成 no-op）。
pub fn install() -> Option<metrics_exporter_prometheus::PrometheusHandle> {
    // 进程内幂等：router() 可能被多次构造（多测试/多实例）——recorder 全局只装一次
    static HANDLE: std::sync::OnceLock<Option<metrics_exporter_prometheus::PrometheusHandle>> =
        std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| {
            if std::env::var("AGENT_MEMORY_METRICS").ok().as_deref() == Some("0") {
                tracing::info!("metrics 已通过 AGENT_MEMORY_METRICS=0 关闭");
                return None;
            }
            match metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder() {
                Ok(handle) => Some(handle),
                Err(e) => {
                    // 重复安装等错误不致命——服务照常，仅无 metrics
                    tracing::warn!(error = %e, "Prometheus recorder 安装失败——/metrics 不可用");
                    None
                }
            }
        })
        .clone()
}

/// GET /metrics：Prometheus 文本 + jobs 队列 gauge（抓取时查库）。端点关闭时 404。
pub async fn metrics_handler(
    axum::extract::Extension(handle): axum::extract::Extension<
        Option<metrics_exporter_prometheus::PrometheusHandle>,
    >,
    axum::extract::Extension(pool): axum::extract::Extension<PgPool>,
) -> Response {
    // jobs 队列深度（R10）：kind × status 计数——抓取驱动，无后台扫描
    if let Ok(rows) = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT kind, status, count(*) FROM jobs GROUP BY kind, status",
    )
    .fetch_all(&pool)
    .await
    {
        for (kind, status, n) in rows {
            metrics::gauge!("jobs_by_status", "kind" => kind, "status" => status).set(n as f64);
        }
    }
    let Some(handle) = handle else {
        return (
            StatusCode::NOT_FOUND,
            "metrics disabled (AGENT_MEMORY_METRICS=0)",
        )
            .into_response();
    };
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        handle.render(),
    )
        .into_response()
}
