//! promote 全链路集成测试（EN-59）：synthesis 页 + 登记行 + 源文档标记双写 +
//! 幂等（重复晋升友好报错）+ promotions 列表。

mod support;

use engram_core::promote::{PromoteRequest, PromoteService};
use engram_core::project::ProjectService;
use sqlx::PgPool;

async fn setup() -> (PgPool, PromoteService, ProjectService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let promote = PromoteService::new(pool.clone());
    let projects = ProjectService::new(pool.clone());
    (pool, promote, projects, container)
}

#[tokio::test]
async fn promote_full_chain_page_registration_and_doc_marks() {
    let (pool, promote, projects, _pg) = setup().await;

    // 源：项目「晋升源」+ 文档《文档工作流》样例
    let proj = projects
        .create_project("晋升源", "dev", Some("用来验证晋升机制的测试项目"))
        .await
        .unwrap();
    let doc = projects
        .add_doc(proj.id, "后端", "", "文档工作流", "机器产出原样透传：KV 逐字存。")
        .await
        .unwrap();

    // ① promote：synthesis 页落库 + 登记 + 源文档标记双写
    let out = promote
        .promote(PromoteRequest {
            project: "晋升源".into(),
            doc_id: doc.id,
            anchor: "§机器产出原样透传".into(),
            slug: "ai-passthrough-principle".into(),
            title: "AI 记系统的透传原则".into(),
            content: "确定性数据 AI 只当管道不当翻译：逐字保存、回读比对。\n\n来源：[[main/ai-passthrough]]".into(),
            library: None,
        })
        .await
        .unwrap();
    assert_eq!(out.library, "main", "缺省目标库 = main");
    assert_eq!(out.page_slug, "ai-passthrough-principle");

    // ② synthesis 页存在：page_type/origin/frontmatter 三对
    let page: (String, String, String) = sqlx::query_as(
        "SELECT page_type, origin, frontmatter->>'promoted_from' FROM wiki_pages \
         WHERE library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main') \
           AND slug = 'ai-passthrough-principle'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(page.0, "synthesis");
    assert_eq!(page.1, "llm");
    assert!(
        page.2.contains(&doc.id.to_string()) && page.2.contains("§机器产出原样透传"),
        "promoted_from 应带源文档 id 与锚点：{page:?}"
    );

    // ③ 登记行存在
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_promotions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // ④ 源文档标记双写可见：frontmatter.promoted 数组 + 正文末尾 ⛳ 行
    let doc_after = projects.get_doc(doc.id).await.unwrap();
    assert!(
        doc_after.frontmatter.to_string().contains("main/ai-passthrough-principle"),
        "frontmatter.promoted 应带 wiki 回链：{:?}",
        doc_after.frontmatter
    );
    assert!(
        doc_after.content.contains("⛳ 本文「§机器产出原样透传」已晋升为 wiki:main/ai-passthrough-principle"),
        "正文末尾应有可见标记行：{}",
        doc_after.content
    );

    // ⑤ 重复晋升同 (project, doc, page) → 友好已存在错误（不重复登记/不覆盖页面）
    let dup = promote
        .promote(PromoteRequest {
            project: "晋升源".into(),
            doc_id: doc.id,
            anchor: "§机器产出原样透传".into(),
            slug: "ai-passthrough-principle".into(),
            title: "AI 记系统的透传原则".into(),
            content: "改版内容".into(),
            library: None,
        })
        .await;
    let err = dup.unwrap_err();
    assert!(
        matches!(err, engram_core::promote::PromoteError::Conflict(ref m) if m.contains("已晋升过")),
        "重复晋升应友好报已晋升：{err:?}"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_promotions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "重复晋升不新增登记");
}

#[tokio::test]
async fn promotions_list_filters_by_project() {
    let (_pool, promote, projects, _pg) = setup().await;
    let proj_a = projects
        .create_project("项目A", "dev", None)
        .await
        .unwrap();
    let proj_b = projects
        .create_project("项目B", "dev", None)
        .await
        .unwrap();
    let doc_a = projects
        .add_doc(proj_a.id, "后端", "", "A 的文档", "内容A")
        .await
        .unwrap();
    let doc_b = projects
        .add_doc(proj_b.id, "后端", "", "B 的文档", "内容B")
        .await
        .unwrap();
    for (proj, doc, slug) in [
        (&proj_a, &doc_a, "from-a"),
        (&proj_b, &doc_b, "from-b"),
    ] {
        promote
            .promote(PromoteRequest {
                project: proj.name.clone(),
                doc_id: doc.id,
                anchor: String::new(),
                slug: slug.into(),
                title: format!("{slug} 标题"),
                content: format!("{slug} 提炼正文"),
                library: None,
            })
            .await
            .unwrap();
    }

    // 按项目过滤：A 只见 A 的登记
    let a_rows = promote.list_promotions(Some("项目A")).await.unwrap();
    assert_eq!(a_rows.len(), 1);
    assert_eq!(a_rows[0].page_slug, "from-a");
    // 全量：两条
    let all = promote.list_promotions(None).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn promote_rejects_cross_project_doc_and_empty_content() {
    let (_pool, promote, projects, _pg) = setup().await;
    projects
        .create_project("甲项目", "dev", None)
        .await
        .unwrap();
    let proj_b = projects
        .create_project("乙项目", "dev", None)
        .await
        .unwrap();
    let doc_b = projects
        .add_doc(proj_b.id, "后端", "", "B 文档", "内容")
        .await
        .unwrap();

    // ① 文档不属于声明的项目 → BadRequest
    let err = promote
        .promote(PromoteRequest {
            project: "甲项目".into(),
            doc_id: doc_b.id,
            anchor: String::new(),
            slug: "cross".into(),
            title: "t".into(),
            content: "c".into(),
            library: None,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, engram_core::promote::PromoteError::BadRequest(ref m) if m.contains("不属于")),
        "{err:?}"
    );

    // ② 空 content（提炼是调用方职责，服务端不代劳）→ BadRequest
    let err = promote
        .promote(PromoteRequest {
            project: "甲项目".into(),
            doc_id: doc_b.id,
            anchor: String::new(),
            slug: "empty".into(),
            title: "t".into(),
            content: "  ".into(),
            library: None,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, engram_core::promote::PromoteError::BadRequest(ref m) if m.contains("提炼")),
        "{err:?}"
    );
}
