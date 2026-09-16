//! engram 服务入口（薄壳：配置 → 池 → 迁移 → 路由 → 监听）。

use std::net::SocketAddr;

use engram_api::config::Config;
use engram_api::routes;
use engram_api::state::AppState;
use engram_storage::PoolConfig;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. 结构化 JSON 日志（必须最先初始化——Config::from_env 的 data_root WARN 依赖它，
    //    放在配置解析之后会让最早的告警静默丢失）
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // 2. 配置
    let cfg = Config::from_env()?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "engram 启动");

    // 3. 数据库连接 + 迁移（启动即跑，失败快速退出）
    let pool = engram_storage::connect_pool(&PoolConfig::new(&cfg.database_url)).await?;
    engram_storage::run_migrations(&pool).await?;
    let version = engram_storage::current_version(&pool).await?;
    tracing::info!(migration_version = ?version, "迁移就绪");

    // 管理员账号播种：账号表为空且 env 密码已设 → username='admin' 播种（兼容旧部署）。
    // env 未设时由登录页初始化表单创建账号——两者都不配则登录不可用（status 端点可见）。
    match engram_core::auth::seed_account_if_empty(&pool, cfg.admin_password.as_deref()).await {
        Ok(true) => tracing::info!("管理员账号已从 env 密码播种（username=admin）——可在设置页修改"),
        Ok(false) => {}
        Err(e) => tracing::warn!("管理员账号播种失败: {e}"),
    }

    // P2：进程崩溃自愈——上次运行中被认领（processing）的会话此刻不可能有
    // extract 在跑，一律退回 pending，否则永远卡死（claim 只取 pending）。
    if let Ok(n) = engram_storage::repo::memory::reset_processing_sessions(&pool).await
        && n > 0
    {
        tracing::info!(n, "启动自愈：processing 会话退回 pending");
    }

    // 4. 任务 runner：注册蒸馏链 + 知识摄取 handler
    // P-C：deep purge 定时执行器——armed 的 job 到期（5 分钟冷却后）真清库。
    let pool_for_purge = pool.clone();
    // R12 per-kind 并发：AGENT_MEMORY_JOB_CONCURRENCY=kind:cap,...（不配则全局并发，行为不变）
    let mut runner_config = engram_jobs::RunnerConfig::default();
    if let Ok(v) = std::env::var("AGENT_MEMORY_JOB_CONCURRENCY") {
        runner_config.per_kind_concurrency = engram_jobs::runner::parse_per_kind_concurrency(&v);
    }
    let runner = engram_distill::register_handlers(
        engram_jobs::Runner::new(pool.clone(), runner_config),
        engram_distill::gateway_llm(
            pool.clone(),
            {
                // 未配置 = 全零占位（纯 chat 场景不碰密钥加密也不出错）；配了坏值则响亮拒绝，
                // 并告诉 AI/人「怎么生成正确的」——此前 expect("主密钥格式恒合法") 在坏值下
                // 打出自相矛盾的日志（恒合法 + NotConfigured + 必须是 64 hex 三信息打架）。
                let master_key_hex =
                    cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32));
                engram_llm::KeyCipher::from_hex_master(&master_key_hex).map_err(|e| {
                    anyhow::anyhow!(
                        "AGENT_MEMORY_MASTER_KEY 非法（{e}）——必须是 64 个 hex 字符（生成：openssl rand -hex 32）；\
                         请修正 ~/.engram/.env 后重启。当前值前 8 字符：{}",
                        &master_key_hex[..master_key_hex.len().min(8)]
                    )
                })?
            },
        ),
    )
    .register("deep_purge", move |_ctx| {
        let pool = pool_for_purge.clone();
        async move {
            let counts = engram_core::purge_deep_pool(&pool)
                .await
                .map_err(|e| engram_jobs::types::JobError::Retryable(e.to_string()))?;
            Ok(counts)
        }
    });
    let runner = engram_cg_bridge::register_handlers(
        runner,
        std::path::PathBuf::from(&cfg.data_dir).join("codegraph"),
    );
    let runner = engram_core::wiki_docs::register_handlers(
        runner,
        engram_llm::ProviderRegistry::new(
            pool.clone(),
            engram_llm::KeyCipher::from_hex_master(
                &cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32)),
            )
            .expect("主密钥格式恒合法"),
        ),
    );
    let runner = engram_core::wiki::ingest::register_handlers(
        runner,
        engram_distill::gateway_llm(
            pool.clone(),
            engram_llm::KeyCipher::from_hex_master(
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
            let wiki = engram_core::wiki::WikiService::new(st.pool.clone(), registry);
            for lib in engram_core::wiki::libraries::list(&st.pool).await {
                match wiki.backfill_tsv(lib.id).await {
                    Ok(n) if n > 0 => {
                        tracing::info!("wiki tsv 存量补数完成：{}（{}）{n} 页", lib.slug, lib.name)
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("wiki tsv 存量补数失败（{} 下次启动重试）: {e}", lib.slug)
                    }
                }
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
