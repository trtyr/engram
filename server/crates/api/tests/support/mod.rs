//! 测试基建：本机 PG 每测试一库（即建即删），不再依赖 Docker/testcontainers。
//!
//! 复用本机 Homebrew PostgreSQL（默认 127.0.0.1:5432，当前系统用户 trust 认证），
//! 每个测试创建唯一命名的临时数据库，Drop 时后台 DROP DATABASE（WITH FORCE）。
//! 隔离性与 testcontainers 等价；可用 AM_TEST_PG_URL 覆盖管理库连接串。

use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};

/// 管理库连接串（建库/删库用；指向已存在的库，通常 postgres）。
fn admin_url() -> String {
    std::env::var("AM_TEST_PG_URL").unwrap_or_else(|_| "postgres://127.0.0.1:5432/postgres".into())
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 测试库守卫：Drop 时后台删库（防累积——残留库可手动 `DROP DATABASE am_test_*`）。
pub struct TestPg {
    pub db_name: String,
}

impl Drop for TestPg {
    fn drop(&mut self) {
        let sql = format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.db_name);
        let _ = std::process::Command::new("psql")
            .arg("-d")
            .arg(admin_url())
            .arg("-c")
            .arg(&sql)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// 建一个干净的测试库（扩展由 run_migrations 的 0001/0009 自建）。
/// 保留旧名以兼容既有调用点；语义 = "启动一个带 pgvector 的 PG 实例"。
pub async fn start_pgvector() -> anyhow::Result<TestPg> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let db_name = format!("am_test_{nanos}_{seq}");

    let admin = sqlx::PgPool::connect(&admin_url()).await?;
    // TEMPLATE template0：并发建库不与 template1 访问冲突（CREATE 会串行取模板锁）
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\" TEMPLATE template0"))
        .execute(&admin)
        .await?;
    admin.close().await;
    Ok(TestPg { db_name })
}

pub async fn connection_url(t: &TestPg) -> anyhow::Result<String> {
    // 与 admin_url 同主机；库名唯一保证测试间隔离
    let admin = admin_url();
    let base = admin.trim_end_matches('/');
    let base = base
        .rsplit_once('/')
        .map(|(host, _)| host.to_string())
        .unwrap_or_else(|| base.to_string());
    Ok(format!("{base}/{}", t.db_name))
}

pub async fn connect_with_retry(url: &str) -> anyhow::Result<PgPool> {
    // 库已建好，正常一次即连；保留重试壳兼容旧签名
    for _ in 0..10 {
        if let Ok(pool) =
            engram_storage::connect_pool(&engram_storage::PoolConfig::new(url)).await
            && sqlx::query("SELECT 1").execute(&pool).await.is_ok()
        {
            return Ok(pool);
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    Err(anyhow::anyhow!("测试库连接超时: {url}"))
}
