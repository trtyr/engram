//! engram 服务入口（薄壳：配置 → 池 → 迁移 → 路由 → 监听）。
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

use std::net::SocketAddr;

use engram_api::config::Config;
use engram_api::routes;
use engram_api::state::AppState;
use engram_storage::PoolConfig;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

/// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
#[allow(clippy::expect_used)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. 结构化 JSON 日志（必须最先初始化——Config::from_env 的 data_root WARN 依赖它，
    //    放在配置解析之后会让最早的告警静默丢失）
    init_tracing();

    // 2. 配置
    let cfg = Config::from_env()?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "engram 启动");

    // 3. 数据库连接 + 迁移 + 管理员播种 + 崩溃自愈（启动即跑，失败快速退出）
    let pool = init_db(&cfg).await?;

    // 4. 任务 runner：注册蒸馏链 + 知识摄取 handler（含 deep purge 定时执行器）
    let runner_handle = build_runner(&cfg, &pool).await?;

    // 5. HTTP 服务
    let state = build_state(&pool, &cfg);
    // W2 存量补数：LLM 页 tsv 曾只嵌 slug，启动时异步重写为 title+content 口径
    spawn_tsv_backfill(&state);
    // L10 + 公网加固：占位/弱熵主密钥显式告警（此状态下创建的 provider 密钥不可迁移）
    warn_master_key(&state);

    let app = routes::router(state).layer(TraceLayer::new_for_http());
    // 6. 监听 + 优雅停机（容器 SIGTERM）：先停 HTTP 再等在途任务
    serve(app, cfg.port, runner_handle).await
}

/// 结构化 JSON 日志初始化（必须最先——配置解析的告警依赖它）。
fn init_tracing() {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

/// 数据库连接 + 迁移 + 管理员播种 + 进程崩溃自愈（processing 会话退回 pending）。
async fn init_db(cfg: &Config) -> anyhow::Result<engram_storage::PgPool> {
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
    Ok(pool)
}

/// 任务 runner：注册蒸馏链 + wiki 摄取 + codegraph/deep_purge handler，返回已启动句柄。
///
/// `expect` 可证安全：未配置主密钥时用 `"00"` × 32 占位（恒 64 hex，格式必然合法）。
#[allow(clippy::expect_used)]
async fn build_runner(
    cfg: &Config,
    pool: &engram_storage::PgPool,
) -> anyhow::Result<engram_jobs::RunnerHandle> {
    // P-C：deep purge 定时执行器——armed 的 job 到期（5 分钟冷却后）真清库。
    let pool_for_purge = pool.clone();
    // R12 per-kind 并发：AGENT_MEMORY_JOB_CONCURRENCY=kind:cap,...（不配则全局并发，行为不变）
    let mut runner_config = engram_jobs::RunnerConfig::default();
    if let Ok(v) = std::env::var("AGENT_MEMORY_JOB_CONCURRENCY") {
        runner_config.per_kind_concurrency = engram_jobs::runner::parse_per_kind_concurrency(&v);
    }
    let distill_llm = engram_distill::gateway_llm(pool.clone(), {
        // 未配置 = 全零占位（纯 chat 场景不碰密钥加密也不出错）；配了坏值则响亮拒绝，
        // 并告诉 AI/人「怎么生成正确的」——此前 expect("主密钥格式恒合法") 在坏值下
        // 打出自相矛盾的日志（恒合法 + NotConfigured + 必须是 64 hex 三信息打架）。
        let master_key_hex = cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32));
        engram_llm::KeyCipher::from_hex_master(&master_key_hex).map_err(|e| {
                anyhow::anyhow!(
                    "AGENT_MEMORY_MASTER_KEY 非法（{e}）——必须是 64 个 hex 字符（生成：openssl rand -hex 32）；\
                     请修正 ~/.engram/.env 后重启。当前值前 8 字符：{}",
                    &master_key_hex[..master_key_hex.len().min(8)]
                )
            })?
    });
    let runner = engram_distill::register_handlers(
        engram_jobs::Runner::new(pool.clone(), runner_config),
        distill_llm.clone(),
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
    let wiki_llm = engram_distill::gateway_llm(
        pool.clone(),
        engram_llm::KeyCipher::from_hex_master(
            &cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32)),
        )
        .expect("主密钥格式恒合法"),
    );
    let runner = engram_core::wiki::ingest::register_handlers(
        runner,
        wiki_llm.clone(),
        engram_core::wiki::WikiService::new(
            pool.clone(),
            engram_llm::ProviderRegistry::new(
                pool.clone(),
                engram_llm::KeyCipher::from_hex_master(
                    &cfg.master_key.clone().unwrap_or_else(|| "00".repeat(32)),
                )
                .expect("主密钥格式恒合法"),
            ),
        )
        // 审计缺陷④：wiki 服务必须持有 LLM 通道——backfill_embeddings（repair job 与
        // 织入尾部的自愈入口）依赖它；此前缺失导致补嵌静默跳过（Ok(0)）
        .with_llm(wiki_llm),
    );
    // 内置节律（roadmap v3）：自续任务 handler + 启动自检补建（幂等键命中即复用，重复启动安全）
    let runner = engram_distill::rhythm::register_rhythm(runner, distill_llm);
    let rhythm_queue = engram_jobs::JobQueue::new(pool.clone());
    if let Err(e) = engram_distill::rhythm::bootstrap(&rhythm_queue, pool).await {
        tracing::warn!("内置节律启动自检失败（不影响启动，下次重启重试）: {e}");
    }
    let runner_handle = runner.start();
    Ok(runner_handle)
}

/// HTTP 服务状态（管理员密码 / 主密钥 / 数据根）。
fn build_state(pool: &engram_storage::PgPool, cfg: &Config) -> AppState {
    AppState::new(pool.clone())
        .with_admin_password(cfg.admin_password.clone())
        .with_master_key(cfg.master_key.clone())
        .with_data_dir(cfg.data_dir.clone())
}

/// W2 存量补数：LLM 页 tsv 口径重写（幂等；失败仅告警不影响服务）。
fn spawn_tsv_backfill(state: &AppState) {
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
}

/// 主密钥健康告警：占位（未设置）与弱熵（模式串，公网下爆破代价近零）。
fn warn_master_key(state: &AppState) {
    if state.is_placeholder_master_key() {
        tracing::warn!(
            "使用占位主密钥（AGENT_MEMORY_MASTER_KEY 未设置）：此时创建的 provider API key 在换用真实主密钥后将无法解密。请尽早设置环境变量；轮换后调用 POST /settings/llm/providers/re-encrypt 迁移存量密钥"
        );
    }
    // 公网加固（P001-t8）：弱熵主密钥告警——已设置但模式化（公网部署下离线爆破代价近零）
    if state.is_weak_master_key() {
        tracing::warn!(
            "主密钥熵过低（64 hex 字符种类 < 8，疑似人手敲的模式串）——公网部署下离线爆破代价接近零。请用 `openssl rand -hex 32` 重新生成，并调用 POST /settings/llm/providers/re-encrypt 迁移存量密钥"
        );
    }
}

/// 监听 + 优雅停机：先等 HTTP 退出，再等在途任务完成（上限 30s，超时交 reap_orphans）。
async fn serve(
    app: axum::Router,
    port: u16,
    runner_handle: engram_jobs::RunnerHandle,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "HTTP 监听");

    // 6. 优雅停机（容器 SIGTERM）：先等 HTTP 再停 runner
    // connect_info：注入连接对端地址（client_ip 中间件读取——活跃会话归因直连场景）
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    // RJ-09 修复：等在途任务完成（上限 30s）再退出——SIGTERM 不再腰斩执行中的 job；
    // 超时未完成者由 reap_orphans 兜底回收（孤儿重排）
    if !runner_handle
        .shutdown_and_wait(std::time::Duration::from_secs(30))
        .await
    {
        tracing::warn!("停机等待超时（30s）：仍有在途任务未完成，交由 reap_orphans 兜底回收");
    }
    Ok(())
}

/// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
#[allow(clippy::expect_used)]
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
