//! 检索集成测试：中文写入→混合检索命中（真 PG + pgvector）。

mod support;

use agent_memory_search::tokenize::tsv_text;
use pgvector::Vector;

#[tokio::test]
async fn chinese_hybrid_search_hits() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    // ≥10 条中文记忆样本
    let samples = [
        ("preference", "用户偏好简洁的中文回答，不要长篇大论"),
        ("fact", "用户住在上海，用 Mac 开发"),
        ("fact", "用户的主要语言是中文"),
        ("decision", "后端选型定为 Rust 而不是 Go"),
        ("convention", "提交信息用中文书写"),
        ("preference", "用户喜欢暗色主题"),
        ("insight", "用户的项目都用 pnpm 管理依赖"),
        ("correction", "用户纠正过：不要用 emoji 回复"),
        ("failure", "直接 rm -rf 根目录被用户严厉批评过"),
        ("event", "2026 年 10 月 agent-memory 项目启动"),
        ("preference", "回答里代码示例要多于解释文字"),
    ];
    for (i, (kind, content)) in samples.iter().enumerate() {
        let emb: Vec<f32> = (0..1024)
            .map(|j| ((i * 37 + j * 13) % 97) as f32 / 97.0)
            .collect();
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, embedding, tsv)
             VALUES ($1, $2, $3, 'active', $4, to_tsvector('simple', $5))",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(kind)
        .bind(content)
        .bind(Vector::from(emb))
        .bind(tsv_text(content))
        .execute(&pool)
        .await
        .unwrap();
    }

    // 纯 FTS：中文关键词命中
    let hits = agent_memory_search::search_atoms(&pool, "用户偏好", None, 5, false, None, None)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "中文 FTS 应有命中");
    assert!(hits.iter().any(|h| h.snippet.contains("偏好")));

    // 纯向量：用一个确定性的向量（与第一条同构）命中
    let probe: Vec<f32> = (0..1024).map(|j| ((j * 13) % 97) as f32 / 97.0).collect();
    let hits = agent_memory_search::search_atoms(&pool, "偏好", Some(&probe), 3, false, None, None)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "向量通道应有命中");

    // 无关查询不应误伤（空命中合法，但这里「Rust 后端」应命中决策条）
    let hits = agent_memory_search::search_atoms(&pool, "后端 选型", None, 5, false, None, None)
        .await
        .unwrap();
    assert!(
        hits.iter().any(|h| h.snippet.contains("Rust")),
        "hits: {:?}",
        hits.iter().map(|h| &h.snippet).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn entity_token_search_prefers_name_hit() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    sqlx::query("INSERT INTO entities (id, name, kind, summary) VALUES ($1, '张三', 'person', '同事，负责后端')")
        .bind(uuid::Uuid::new_v4()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO entities (id, name, kind, summary) VALUES ($1, '李四', 'person', '骑行爱好者，周末环湖')")
        .bind(uuid::Uuid::new_v4()).execute(&pool).await.unwrap();

    // 名字命中：搜「张三」只给张三
    let hits = agent_memory_search::search_entities(&pool, "张三", 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title.as_deref(), Some("张三"));
    assert_eq!(hits[0].kind.as_deref(), Some("person"));

    // 摘要命中：搜「骑行」给李四（弱命中也是命中）
    let hits = agent_memory_search::search_entities(&pool, "骑行", 5)
        .await
        .unwrap();
    assert!(hits.iter().any(|h| h.title.as_deref() == Some("李四")));

    // 无命中：空结果
    let hits = agent_memory_search::search_entities(&pool, "王五", 5)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

/// phase-2 过期降权：valid_until 已过的原子 score 减半排后（不消失，历史价值还在）。
#[tokio::test]
async fn search_demotes_expired_atoms() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    // 同题材两条（同 content 保证基础分相同）：一条过期、一条未过期
    // search crate 无 chrono 依赖，valid_until 用 SQL now()±interval 表达
    let expired_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, tsv, valid_until) \
         VALUES ($1, 'fact', '下周三要交周报', 'active', to_tsvector('simple', $2), now() - interval '1 day')",
    )
    .bind(expired_id)
    .bind(tsv_text("下周三要交周报"))
    .execute(&pool)
    .await
    .unwrap();
    let fresh_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, tsv, valid_until) \
         VALUES ($1, 'fact', '下周三要交周报', 'active', to_tsvector('simple', $2), now() + interval '1 day')",
    )
    .bind(fresh_id)
    .bind(tsv_text("下周三要交周报"))
    .execute(&pool)
    .await
    .unwrap();

    let hits = agent_memory_search::search_atoms(&pool, "周报", None, 10, false, None, None)
        .await
        .unwrap();
    let score_of = |id: uuid::Uuid| {
        hits.iter()
            .find(|h| h.id == id)
            .map(|h| h.score)
            .unwrap_or(f64::NAN)
    };
    assert!(
        score_of(expired_id) < score_of(fresh_id),
        "过期原子应降权（score 更低），实得 expired={} fresh={}",
        score_of(expired_id),
        score_of(fresh_id)
    );
}

/// phase-2 时间范围过滤：from/to 过滤（occurred_at 优先 NULL fallback created_at）。
#[tokio::test]
async fn search_filters_by_time_range() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    // 三条同题材原子，occurred_at 分处 8/9/10 月
    for month in ["2026-08-15", "2026-09-15", "2026-10-15"] {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, tsv, occurred_at) \
             VALUES ($1, 'event', '时间过滤测试事件', 'active', to_tsvector('simple', $2), $3::timestamptz)",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(tsv_text("时间过滤测试事件"))
        .bind(format!("{month}T00:00:00Z"))
        .execute(&pool)
        .await
        .unwrap();
    }

    let from: chrono::DateTime<chrono::Utc> =
        chrono::DateTime::parse_from_rfc3339("2026-08-15T00:00:00Z")
            .unwrap()
            .into();
    let to: chrono::DateTime<chrono::Utc> =
        chrono::DateTime::parse_from_rfc3339("2026-09-30T00:00:00Z")
            .unwrap()
            .into();
    let hits = agent_memory_search::search_atoms(
        &pool,
        "时间过滤测试",
        None,
        20,
        false,
        Some(from),
        Some(to),
    )
    .await
    .unwrap();
    assert_eq!(hits.len(), 2, "8/15~9/30 窗内应命中 8 月和 9 月两条");
}
