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

    // P2：进程崩溃自愈——上次运行中被认领（processing）的会话此刻不可能有
    // extract 在跑，一律退回 pending，否则永远卡死（claim 只取 pending）。
    if let Ok(n) = sqlx::query(
        "UPDATE raw_sessions SET distill_status = 'pending' WHERE distill_status = 'processing'",
    )
    .execute(&pool)
    .await
        && n.rows_affected() > 0
    {
        tracing::info!(
            n = n.rows_affected(),
            "启动自愈：processing 会话退回 pending"
        );
    }

    // 4. 任务 runner：注册蒸馏链 + 知识摄取 handler
    // P-C：deep purge 定时执行器——armed 的 job 到期（5 分钟冷却后）真清库。
    let pool_for_purge = pool.clone();
    let runner = agent_memory_distill::register_handlers(
        agent_memory_jobs::Runner::new(pool.clone(), agent_memory_jobs::RunnerConfig::default()),
        agent_memory_distill::gateway_llm(
            pool.clone(),
            agent_memory_llm::KeyCipher::from_hex_master(
                &cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32)),
            )
            .expect("主密钥格式恒合法"),
        ),
    )
    .register("deep_purge", move |_ctx| {
        let pool = pool_for_purge.clone();
        async move {
            let counts = agent_memory_core::purge_deep_pool(&pool)
                .await
                .map_err(|e| agent_memory_jobs::types::JobError::Retryable(e.to_string()))?;
            Ok(counts)
        }
    });
    let runner = agent_memory_core::knowledge::register_handlers(
        runner,
        agent_memory_llm::ProviderRegistry::new(
            pool.clone(),
            agent_memory_llm::KeyCipher::from_hex_master(
                &cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32)),
            )
            .expect("主密钥格式恒合法"),
        ),
    );
    let runner = agent_memory_core::wiki::ingest::register_handlers(
        runner,
        agent_memory_distill::gateway_llm(
            pool.clone(),
            agent_memory_llm::KeyCipher::from_hex_master(
                &cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32)),
            )
            .expect("主密钥格式恒合法"),
        ),
    );
    let runner_handle = runner.start();

    // 5. HTTP 服务
    let state = AppState::new(pool)
        .with_admin_password(cfg.admin_password.clone())
        .with_master_key(cfg.master_key.clone())
        .with_data_dir(cfg.data_dir.clone());

    // W2 存量补数：LLM 页 tsv 曾只嵌 slug，启动时异步重写为 title+content 口径
    // （幂等：值不变不写；失败仅告警不影响服务）
    {
        let st = state.clone();
        tokio::spawn(async move {
            let registry = st.registry();
            let wiki = agent_memory_core::wiki::WikiService::new(st.pool.clone(), registry);
            match wiki.backfill_tsv().await {
                Ok(n) if n > 0 => tracing::info!("wiki tsv 存量补数完成：{n} 页"),
                Ok(_) => {}
                Err(e) => tracing::warn!("wiki tsv 存量补数失败（下次启动重试）: {e}"),
            }
        });
    }

    // L10：占位主密钥显式告警——此状态下创建的 provider 密钥与后续真实密钥不兼容
    if state.is_placeholder_master_key() {
        tracing::warn!(
            "使用占位主密钥（AGENT_MEMORY_MASTER_KEY 未设置）：此时创建的 provider API key 在换用真实主密钥后将无法解密。请尽早设置环境变量；轮换后调用 POST /settings/llm/providers/re-encrypt 迁移存量密钥"
        );
    }

    let app = routes::router(state).layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "HTTP 监听");

    // 6. 优雅停机（容器 SIGTERM）：先等 HTTP 再停 runner
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    runner_handle.shutdown();
    runner_handle.join().await;
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
