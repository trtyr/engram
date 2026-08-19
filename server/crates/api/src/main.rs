//! agent-memory 服务入口（薄壳：配置 → 池 → 迁移 → 路由 → 监听）。

use std::net::SocketAddr;

use agent_memory_api::config::Config;
use agent_memory_api::routes;
use agent_memory_api::state::AppState;
use agent_memory_storage::PoolConfig;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. 配置
    let cfg = Config::from_env()?;

    // 2. 结构化 JSON 日志
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "agent-memory 启动");

    // 3. 数据库连接 + 迁移（启动即跑，失败快速退出）
    let pool = agent_memory_storage::connect_pool(&PoolConfig::new(&cfg.database_url)).await?;
    agent_memory_storage::run_migrations(&pool).await?;
    let version = agent_memory_storage::current_version(&pool).await?;
    tracing::info!(migration_version = ?version, "迁移就绪");

    // 4. 任务 runner（Phase 1：占位注册表；Phase 2 起注册真实 handler）
    // TODO(phase-2): Runner::new(pool.clone(), RunnerConfig::default()).register(...).start()

    // 5. HTTP 服务
    let state = AppState::new(pool)
        .with_admin_password(cfg.admin_password.clone())
        .with_master_key(cfg.master_key.clone());
    let app = routes::router(state).layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "HTTP 监听");

    // 6. 优雅停机（容器 SIGTERM）
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("收到停机信号，优雅退出");
}
