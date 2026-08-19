//! 知识域集成测试：摄取管道全链路 + SSRF 拒绝集 + 幂等 + 容错。

mod support;

use agent_memory_core::knowledge::ssrf::{FetchError, is_private_ip, safe_fetch};
use std::net::IpAddr;
use std::time::Duration;

// ---------- SSRF 单元 ----------

#[test]
fn ssrf_ip_classification() {
    let priv_v4 = [
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "240.0.0.1",
        "192.0.2.1",
        "198.18.0.1",
    ];
    for ip in priv_v4 {
        assert!(
            is_private_ip(ip.parse::<IpAddr>().unwrap()),
            "{ip} 应为私网"
        );
    }
    let pub_v4 = ["8.8.8.8", "1.1.1.1", "82.157.147.224"];
    for ip in pub_v4 {
        assert!(
            !is_private_ip(ip.parse::<IpAddr>().unwrap()),
            "{ip} 应为公网"
        );
    }
    let priv_v6 = ["::1", "fe80::1", "fc00::1", "ff02::1", "::ffff:127.0.0.1"];
    for ip in priv_v6 {
        assert!(
            is_private_ip(ip.parse::<IpAddr>().unwrap()),
            "{ip} 应为私网"
        );
    }
    assert!(
        !is_private_ip("2606:4700:4700::1111".parse::<IpAddr>().unwrap()),
        "公网 v6 应放行"
    );
}

#[tokio::test]
async fn ssrf_fetch_rejects_private_targets() {
    // 环回
    let e = safe_fetch("http://127.0.0.1:1/x", 1024, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::PrivateAddress), "{e:?}");
    // 链路本地（云元数据端点）
    let e = safe_fetch(
        "http://169.254.169.254/latest/meta-data",
        1024,
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert!(matches!(e, FetchError::PrivateAddress), "{e:?}");
    // 内网段
    let e = safe_fetch(
        "http://10.2.0.14:3000/v1/models",
        1024,
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert!(matches!(e, FetchError::PrivateAddress), "{e:?}");
    // 非 http 协议
    let e = safe_fetch("file:///etc/passwd", 1024, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::Scheme), "{e:?}");
    // localhost 域名（解析到环回）
    let e = safe_fetch("http://localhost:8080/health", 1024, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::PrivateAddress), "{e:?}");
}

// ---------- 摄取管道（真 PG + 本地 mock embed 不可行——嵌入降级路径） ----------

use agent_memory_core::knowledge::{IngestSource, KnowledgeService};
use agent_memory_llm::{KeyCipher, ProviderRegistry};

async fn setup() -> (sqlx::PgPool, KnowledgeService) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    std::mem::forget(container);
    let dir = tempfile::tempdir().unwrap();
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let svc = KnowledgeService::new(pool.clone(), registry, dir.keep());
    (pool, svc)
}

/// 起 Runner 跑知识管道（无 LLM provider → embedding 全降级 FTS，仍 ready）。
async fn run_jobs(pool: sqlx::PgPool) -> agent_memory_jobs::RunnerHandle {
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let runner = agent_memory_core::knowledge::register_handlers(
        agent_memory_jobs::Runner::new(
            pool,
            agent_memory_jobs::RunnerConfig {
                worker_id: "test".into(),
                concurrency: 2,
                poll_interval: Duration::from_millis(20),
                batch_size: 10,
                reap_interval: Duration::from_secs(3600),
            },
        ),
        registry,
    );
    runner.start()
}

async fn wait_ready(
    svc: &KnowledgeService,
    id: uuid::Uuid,
) -> agent_memory_core::knowledge::DocumentDto {
    for _ in 0..300 {
        if let Ok(doc) = svc.get_document(id).await
            && matches!(doc.status.as_str(), "ready" | "failed")
        {
            return doc;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("文档 {id} 摄取超时");
}

#[tokio::test]
async fn md_ingest_to_ready_and_chinese_fts_search() {
    let (pool, svc) = setup().await;
    let handle = run_jobs(pool.clone()).await;

    let md = format!(
        "# Rust 记忆系统\n\nagent-memory 是一个用 Rust 编写的长期记忆平台。\n\n# 部署方式\n\n通过 Docker Compose 一键部署，PostgreSQL 搭配 pgvector 扩展。\n\n{}",
        "补充细节。".repeat(200)
    );
    let md_bytes = md.into_bytes();
    let (id, deduped) = svc
        .submit(IngestSource::Bytes {
            name: "memory-guide.md".into(),
            content: md_bytes.clone(),
            content_type: Some("text/markdown".into()),
        })
        .await
        .unwrap();
    assert!(!deduped);

    let doc = wait_ready(&svc, id).await;
    assert_eq!(doc.status, "ready", "error: {:?}", doc.error);

    // 分块存在
    let chunks = svc.chunks(id, 500).await.unwrap();
    assert!(chunks.len() >= 2, "长文应分多块: {}", chunks.len());

    // 中文 FTS 检索命中（无 embedding 通道 → 纯 FTS）
    let hits = svc.search("Rust 记忆平台", 5).await.unwrap();
    assert!(!hits.is_empty(), "中文检索应有命中");
    assert!(hits[0].snippet.contains("Rust") || hits[0].snippet.contains("记忆"));
    assert_eq!(hits[0].document_id, id, "命中应带文档引用");

    // 重复上传 → 幂等秒回
    let (id2, deduped2) = svc
        .submit(IngestSource::Bytes {
            name: "memory-guide.md".into(),
            content: md_bytes,
            content_type: Some("text/markdown".into()),
        })
        .await
        .unwrap();
    assert!(deduped2);
    assert_eq!(id, id2);

    handle.shutdown();
    handle.join().await;
}

#[tokio::test]
async fn html_ingest_and_corrupt_file_not_blocking() {
    let (pool, svc) = setup().await;
    let handle = run_jobs(pool.clone()).await;

    // HTML 摄取
    let html = "<html><head><title>知识图谱指南</title><style>.x{}</style></head><body><h1>知识图谱</h1><p>知识图谱把代码符号与调用关系组织成图结构。</p><script>alert(1)</script></body></html>";
    let (h_id, _) = svc
        .submit(IngestSource::Bytes {
            name: "graph.html".into(),
            content: html.as_bytes().to_vec(),
            content_type: Some("text/html".into()),
        })
        .await
        .unwrap();
    let doc = wait_ready(&svc, h_id).await;
    assert_eq!(doc.status, "ready");

    let hits = svc.search("知识图谱 调用关系", 5).await.unwrap();
    assert!(!hits.is_empty());
    assert!(
        !hits.iter().any(|h| h.snippet.contains("alert")),
        "script 不应入库"
    );

    // 损坏 PDF：failed 但不崩
    let (bad_id, _) = svc
        .submit(IngestSource::Bytes {
            name: "broken.pdf".into(),
            content: b"this is not a pdf".to_vec(),
            content_type: Some("application/pdf".into()),
        })
        .await
        .unwrap();
    let bad = wait_ready(&svc, bad_id).await;
    assert_eq!(bad.status, "failed", "损坏文件应 failed: {:?}", bad.error);

    // 队列不阻塞：后续任务照常
    let (ok_id, _) = svc
        .submit(IngestSource::Bytes {
            name: "after.md".into(),
            content: "# 正常文档\n\n内容正常".as_bytes().to_vec(),
            content_type: None,
        })
        .await
        .unwrap();
    let ok = wait_ready(&svc, ok_id).await;
    assert_eq!(ok.status, "ready");

    handle.shutdown();
    handle.join().await;
}
