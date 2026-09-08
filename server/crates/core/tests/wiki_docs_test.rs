//! wiki 文档域集成测试：摄取管道全链路 + SSRF 拒绝集 + 幂等 + 容错。

mod support;

use engram_core::wiki_docs::ssrf::{FetchError, is_private_ip, safe_fetch, safe_fetch_opts};
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

// ---------- K3：代理模式私网校验保留 ----------

#[tokio::test]
async fn ssrf_proxy_mode_still_rejects_private() {
    // via_proxy=true 时不再 pin，但解析+私网校验必须仍然生效
    for url in [
        "http://127.0.0.1:1/x",
        "http://10.0.0.5/y",
        "http://169.254.169.254/meta",
        "http://localhost:9/z",
    ] {
        let e = safe_fetch_opts(url, 1024, Duration::from_secs(2), true)
            .await
            .unwrap_err();
        assert!(
            matches!(e, FetchError::PrivateAddress),
            "代理模式 {url} 仍应拒私网: {e:?}"
        );
    }
    // 非 http 协议同样保留
    let e = safe_fetch_opts("file:///etc/passwd", 1024, Duration::from_secs(2), true)
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::Scheme), "{e:?}");
}

// ---------- 摄取管道（真 PG + 本地 mock embed 不可行——嵌入降级路径） ----------

use engram_core::wiki_docs::{IngestSource, WikiDocumentService};
use engram_llm::{KeyCipher, ProviderRegistry};

async fn setup() -> (sqlx::PgPool, WikiDocumentService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let dir = tempfile::tempdir().unwrap();
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let svc = WikiDocumentService::new(pool.clone(), registry, dir.keep());
    (pool, svc, container)
}

/// 默认主库 id（0037 多库：wiki_documents/wiki_chunks 的 library_id NOT NULL）。
async fn main_lib(pool: &sqlx::PgPool) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 起 Runner 跑知识管道（无 LLM provider → embedding 全降级 FTS，仍 ready）。
async fn run_jobs(pool: sqlx::PgPool) -> engram_jobs::RunnerHandle {
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let runner = engram_core::wiki_docs::register_handlers(
        engram_jobs::Runner::new(
            pool,
            engram_jobs::RunnerConfig {
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
    svc: &WikiDocumentService,
    lib: uuid::Uuid,
    id: uuid::Uuid,
) -> engram_core::wiki_docs::DocumentDto {
    for _ in 0..300 {
        if let Ok(doc) = svc.get_document(lib, id).await
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
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    let handle = run_jobs(pool.clone()).await;

    let md = format!(
        "# Rust 记忆系统\n\nengram 是一个用 Rust 编写的长期记忆平台。\n\n# 部署方式\n\n通过 Docker Compose 一键部署，PostgreSQL 搭配 pgvector 扩展。\n\n{}",
        "补充细节。".repeat(200)
    );
    let md_bytes = md.into_bytes();
    let (id, deduped) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "memory-guide.md".into(),
                content: md_bytes.clone(),
                content_type: Some("text/markdown".into()),
            },
        )
        .await
        .unwrap();
    assert!(!deduped);

    let doc = wait_ready(&svc, lib, id).await;
    assert_eq!(doc.status, "ready", "error: {:?}", doc.error);

    // 分块存在
    let chunks = svc.chunks(lib, id, 500).await.unwrap();
    assert!(chunks.len() >= 2, "长文应分多块: {}", chunks.len());

    // 中文 FTS 检索命中（无 embedding 通道 → 纯 FTS）
    let hits = svc.search(lib, "Rust 记忆平台", 5).await.unwrap();
    assert!(!hits.is_empty(), "中文检索应有命中");
    assert!(hits[0].snippet.contains("Rust") || hits[0].snippet.contains("记忆"));
    assert_eq!(hits[0].document_id, id, "命中应带文档引用");

    // 重复上传 → 幂等秒回
    let (id2, deduped2) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "memory-guide.md".into(),
                content: md_bytes,
                content_type: Some("text/markdown".into()),
            },
        )
        .await
        .unwrap();
    assert!(deduped2);
    assert_eq!(id, id2);

    handle.shutdown();
    handle.join().await;
}

#[tokio::test]
async fn html_ingest_and_corrupt_file_not_blocking() {
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    let handle = run_jobs(pool.clone()).await;

    // HTML 摄取
    let html = "<html><head><title>知识图谱指南</title><style>.x{}</style></head><body><h1>知识图谱</h1><p>知识图谱把代码符号与调用关系组织成图结构。</p><script>alert(1)</script></body></html>";
    let (h_id, _) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "graph.html".into(),
                content: html.as_bytes().to_vec(),
                content_type: Some("text/html".into()),
            },
        )
        .await
        .unwrap();
    let doc = wait_ready(&svc, lib, h_id).await;
    assert_eq!(doc.status, "ready");

    let hits = svc.search(lib, "知识图谱 调用关系", 5).await.unwrap();
    assert!(!hits.is_empty());
    assert!(
        !hits.iter().any(|h| h.snippet.contains("alert")),
        "script 不应入库"
    );

    // 损坏 PDF：failed 但不崩
    let (bad_id, _) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "broken.pdf".into(),
                content: b"this is not a pdf".to_vec(),
                content_type: Some("application/pdf".into()),
            },
        )
        .await
        .unwrap();
    let bad = wait_ready(&svc, lib, bad_id).await;
    assert_eq!(bad.status, "failed", "损坏文件应 failed: {:?}", bad.error);

    // 队列不阻塞：后续任务照常
    let (ok_id, _) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "after.md".into(),
                content: "# 正常文档\n\n内容正常".as_bytes().to_vec(),
                content_type: None,
            },
        )
        .await
        .unwrap();
    let ok = wait_ready(&svc, lib, ok_id).await;
    assert_eq!(ok.status, "ready");

    handle.shutdown();
    handle.join().await;
}

// ---------- K6：并发同 sha 提交（提交路径正确性） ----------

#[tokio::test]
async fn concurrent_same_sha_submit_is_idempotent() {
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    // 不起 Runner：只验证提交路径本身的并发语义
    let content = b"# concurrent dedup test\nsame bytes here".to_vec();
    let mk = || IngestSource::Bytes {
        name: "dup.md".into(),
        content: content.clone(),
        content_type: Some("text/markdown".into()),
    };

    let s2 = svc.clone();
    let (a, b) = tokio::join!(
        svc.submit(lib, mk()),
        async move { s2.submit(lib, mk()).await }
    );
    let (id_a, dup_a) = a.expect("A 应成功");
    let (id_b, dup_b) = b.expect("B 应成功");
    assert_eq!(id_a, id_b, "并发同 sha 返回同一 id");
    assert!(dup_a ^ dup_b, "恰一方幂等命中（a={dup_a}, b={dup_b}）");

    // 串行重复 → 幂等命中
    let (id3, dup3) = svc.submit(lib, mk()).await.unwrap();
    assert_eq!(id3, id_a);
    assert!(dup3);

    // documents 只一行；parse job 只一个
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wiki_documents")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "同 sha 只留一行");
    let jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE kind = 'parse_document' AND payload->>'document_id' = $1",
    )
    .bind(id_a.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(jobs, 1, "只有一个 parse job");

    // 冲突方清理了自己刚写的文件副本，落盘只剩赢家一份
    let files: Vec<String> = std::fs::read_dir(svc.data_dir.join("uploads"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.contains("dup.md"))
        .collect();
    assert_eq!(files.len(), 1, "冲突路径文件已清理: {files:?}");
}

// ---------- K1：failed 文档重提交自愈 ----------

#[tokio::test]
async fn failed_doc_resubmit_self_heals() {
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    let handle = run_jobs(pool.clone()).await;

    // 预置：曾抓取失败的文档（failed + error），但原始文件在盘上
    let name = "heal.md";
    let ct = Some("text/markdown".to_string());
    let content = "# 自愈测试文档\n失败之后重新提交应当走完全链路。\n\n补充段落内容。".repeat(20);
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(name.as_bytes());
    hasher.update(ct.as_deref().unwrap_or("").as_bytes());
    hasher.update(content.as_bytes());
    let sha: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let doc_id = uuid::Uuid::now_v7();
    let path = svc
        .data_dir
        .join("uploads")
        .join(format!("{doc_id}_{name}"));
    std::fs::create_dir_all(svc.data_dir.join("uploads")).unwrap();
    std::fs::write(&path, content.as_bytes()).unwrap();
    sqlx::query(
        "INSERT INTO wiki_documents (id, library_id, title, source_uri, mime, raw_path, sha256, status, error) \
         VALUES ($1, $2, $3, $3, $4, $5, $6, 'failed', 'URL 抓取失败: 网络抖动')",
    )
    .bind(doc_id)
    .bind(lib)
    .bind(name)
    .bind(&ct)
    .bind(path.to_string_lossy().as_ref())
    .bind(&sha)
    .execute(&pool)
    .await
    .unwrap();

    // 重新提交同 sha → 幂等命中 + 自愈重置 + 重新入队
    let (rid, deduped) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: name.into(),
                content: content.into_bytes(),
                content_type: ct,
            },
        )
        .await
        .unwrap();
    assert_eq!(rid, doc_id, "幂等命中既有文档");
    assert!(deduped, "自愈走幂等语义（200）而非新建");

    // 走完整链路直至 ready
    let doc = wait_ready(&svc, lib, doc_id).await;
    assert_eq!(doc.status, "ready", "自愈后应走完全链路: {:?}", doc.error);
    assert!(doc.error.is_none());
    let chunks = svc.chunks(lib, doc_id, 500).await.unwrap();
    assert!(!chunks.is_empty(), "块应已生成");

    handle.shutdown();
    handle.join().await;
}

#[tokio::test]
async fn ready_doc_resubmit_does_not_reingest() {
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    let handle = run_jobs(pool.clone()).await;

    let content = "# 就绪文档\n不再重摄取".repeat(10);
    let (id, _) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "stable.md".into(),
                content: content.clone().into_bytes(),
                content_type: Some("text/markdown".into()),
            },
        )
        .await
        .unwrap();
    let doc = wait_ready(&svc, lib, id).await;
    assert_eq!(doc.status, "ready");

    // ready 后重提交 → 不自愈、不入队（jobs 数不变）
    let _jobs_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (rid, deduped) = svc
        .submit(
            lib,
            IngestSource::Bytes {
                name: "stable.md".into(),
                content: content.into_bytes(),
                content_type: Some("text/markdown".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(rid, id);
    assert!(deduped);
    handle.shutdown();
    handle.join().await;
}

// ---------- K4/K8：嵌入链（只补缺失 + 短响应守卫 + re-embed 端点） ----------

async fn insert_ready_doc_with_chunks(
    pool: &sqlx::PgPool,
    lib: uuid::Uuid,
    with_embedding: bool,
) -> uuid::Uuid {
    let doc_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_documents (id, library_id, title, source_uri, sha256, status) \
         VALUES ($1, $2, '补嵌测试', 'reembed.md', $3, 'ready')",
    )
    .bind(doc_id)
    .bind(lib)
    .bind(format!("sha-reembed-{}", uuid::Uuid::now_v7().simple()))
    .execute(pool)
    .await
    .unwrap();
    for seq in 0..2 {
        sqlx::query(
            "INSERT INTO wiki_chunks (id, library_id, document_id, seq, content, embed_failed, embedding, tsv) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, to_tsvector('simple', $5))",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(lib)
        .bind(doc_id)
        .bind(seq)
        .bind(format!("补嵌测试第 {seq} 段内容，关于 Rust 向量检索。"))
        // seq=0 已嵌入（或 NULL），seq=1 恒为缺失
        .bind(seq == 1)
        .bind(if with_embedding && seq == 0 {
            Some(pgvector::Vector::from(vec![0.5f32; 1024]))
        } else {
            None
        })
        .execute(pool)
        .await
        .unwrap();
    }
    doc_id
}

/// 等待指定文档的某类 job 到终态（文档本身可能已 ready，不能靠 status 轮询）。
async fn wait_job_done(pool: &sqlx::PgPool, doc_id: uuid::Uuid, kind: &str) -> String {
    for _ in 0..300 {
        if let Some(status) = sqlx::query_scalar::<_, String>(
            "SELECT status FROM jobs \
             WHERE kind = $1 AND payload->>'document_id' = $2 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(kind)
        .bind(doc_id.to_string())
        .fetch_optional(pool)
        .await
        .unwrap()
            && matches!(status.as_str(), "succeeded" | "failed" | "dead")
        {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job {kind} for {doc_id} 超时");
}

#[tokio::test]
async fn reembed_only_touches_missing_chunks() {
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    let handle = run_jobs(pool.clone()).await;

    let doc_id = insert_ready_doc_with_chunks(&pool, lib, true).await;

    // re-embed（无 provider → 缺失块维持降级，但已嵌入块绝不被碰）
    svc.reembed(lib, doc_id).await.unwrap();
    let st = wait_job_done(&pool, doc_id, "embed_document").await;
    assert_eq!(st, "succeeded");
    let doc = svc.get_document(lib, doc_id).await.unwrap();
    assert_eq!(doc.status, "ready");

    let rows: Vec<(bool, bool)> = sqlx::query_as(
        "SELECT embed_failed, (embedding IS NOT NULL) FROM wiki_chunks \
         WHERE document_id = $1 ORDER BY seq",
    )
    .bind(doc_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (false, true), "已嵌入块不被重复处理");
    assert_eq!(rows[1], (true, false), "缺失块保持降级标志（无 provider）");

    // 非 ready 文档拒绝 re-embed
    let pending_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_documents (id, library_id, title, source_uri, sha256, status) \
         VALUES ($1, $2, '未就绪', 'x.md', $3, 'pending')",
    )
    .bind(pending_id)
    .bind(lib)
    .bind(format!("sha-pending-{}", uuid::Uuid::now_v7().simple()))
    .execute(&pool)
    .await
    .unwrap();
    let err = svc.reembed(lib, pending_id).await.unwrap_err();
    assert!(
        matches!(
            err,
            engram_core::wiki_docs::WikiDocumentError::BadRequest(_)
        ),
        "{err:?}"
    );
    // 不存在的文档 → NotFound
    let err = svc.reembed(lib, uuid::Uuid::now_v7()).await.unwrap_err();
    assert!(
        matches!(err, engram_core::wiki_docs::WikiDocumentError::NotFound(_)),
        "{err:?}"
    );

    handle.shutdown();
    handle.join().await;
}

// ---------- K7：空 token 查询短路 ----------

#[tokio::test]
async fn k7_empty_token_query_returns_empty_not_error() {
    let (pool, svc, _pg) = setup().await; // 无 provider → 无查询向量 → 守卫短路
    let lib = main_lib(&pool).await;
    for q in ["书", "的", "??", "a", "  "] {
        let hits = svc.search(lib, q, 5).await.unwrap();
        assert!(hits.is_empty(), "「{q}」应短路返回空而非空跑 FTS");
    }
}

#[tokio::test]
async fn embed_short_response_marks_batch_failed_not_silent_null() {
    let (pool, svc, _pg) = setup().await;
    let lib = main_lib(&pool).await;
    let handle = run_jobs(pool.clone()).await;

    // 本地 mock 嵌入网关：对任意输入只回 1 条向量（短响应）
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let emb = format!("[{}]", vec!["0.5"; 1024].join(","));
        let body = format!(
            r#"{{"data":[{{"index":0,"embedding":{emb}}}],"usage":{{"prompt_tokens":2,"total_tokens":2}}}}"#
        );
        for _ in 0..4 {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let mut got = String::new();
            // 简易读请求：读到空行 + body（Content-Length 前缀足够小，一次读大概率够）
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.push_str(&String::from_utf8_lossy(&buf[..n]));
                if got.contains("\r\n\r\n") && got.len() > 200 {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });

    // 注册 mock 为默认 provider（registry 每次调用都读库）
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    let key_enc = cipher.encrypt("mock-key").unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default) \
         VALUES ($1, 'mock-embed', $2, $3, 'mock-model', 'embedding', true)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(format!("http://127.0.0.1:{port}"))
    .bind(&key_enc)
    .execute(&pool)
    .await
    .unwrap();

    // 2 个缺失块 → 批次 2 条输入、响应只回 1 条 → K4 守卫应整批降级
    let doc_id = insert_ready_doc_with_chunks(&pool, lib, false).await;
    svc.reembed(lib, doc_id).await.unwrap();
    let st = wait_job_done(&pool, doc_id, "embed_document").await;
    assert_eq!(st, "succeeded", "短响应降级不是 job 失败");
    let doc = svc.get_document(lib, doc_id).await.unwrap();
    assert_eq!(doc.status, "ready", "短响应降级不阻塞 ready");

    let rows: Vec<(bool, bool)> = sqlx::query_as(
        "SELECT embed_failed, (embedding IS NOT NULL) FROM wiki_chunks \
         WHERE document_id = $1 ORDER BY seq",
    )
    .bind(doc_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    for (failed, has_vec) in &rows {
        assert!(failed, "短响应批次应标 embed_failed");
        assert!(!has_vec, "不应写入任何向量（含 NULL+false 双静默的旧路径）");
    }

    handle.shutdown();
    handle.join().await;
}
