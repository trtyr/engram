//! projects doc_search 检索质量集成测试（0042 检索升级；工单「文档检索可用性」验收）：
//! 宽泛词首屏定位 / 多词查询 / 归一化召回 / 边界。

mod support;

use engram_core::project::ProjectService;
use sqlx::PgPool;

async fn setup() -> (PgPool, ProjectService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let svc = ProjectService::new(pool.clone());
    (pool, svc, container)
}

/// 验收①：宽泛词「部署」的首屏应定位到「运行与部署」文档（title 加权 + 命中密度），
/// 而不是按文档创建顺序被无关长文泡掉。
#[tokio::test]
async fn broad_query_surfaces_target_doc_first() {
    let (_pool, svc, _pg) = setup().await;
    let p = svc
        .create_project("检索质量", "dev", None)
        .await
        .expect("建项目");

    // 无关长文（先建——旧实现按创建顺序扫，它的高频命中会占满首屏）
    let filler: String = (0..40).map(|_| "项目记忆需持续蒸馏与整理。\n").collect();
    svc.add_doc(p.id, "部署", "", "蒸馏随笔", &filler)
        .await
        .unwrap();
    // 目标文档：title 含「部署」+ 正文多行部署 + 含端口行
    svc.add_doc(
        p.id,
        "部署",
        "",
        "运行与部署",
        "# 运行与部署\n\n本地起栈：cargo run 即可。\nDocker 部署走 compose：pg17 双容器。\n部署端口 17654，容器内 8080。\n部署密钥见 .env。\n",
    )
    .await
    .unwrap();
    // 另一篇弱相关（只 1 行提到）
    svc.add_doc(
        p.id,
        "部署",
        "",
        "产品定位",
        "# 产品定位\n\n支持 Docker 单机部署。\n",
    )
    .await
    .unwrap();

    let hits = svc.search_doc_lines(p.id, "部署", 50).await.unwrap();
    assert!(!hits.is_empty(), "应有命中");
    assert_eq!(
        hits[0].title,
        "运行与部署",
        "首屏第一命中应来自目标文档（title 加权 + 密度）: {:#?}",
        &hits[..3]
    );
    // 端口行应在前几条（该行含「部署」+「端口」两词）
    let port_pos = hits
        .iter()
        .position(|h| h.text.contains("17654"))
        .expect("端口行应命中");
    assert!(port_pos < 10, "端口行应靠前（位置 {port_pos}）");
    // score/doc_hit_count 字段
    assert!(hits[0].score > 0 && hits[0].doc_hit_count >= 1);
}

/// 验收③：多词查询「部署 端口」——两词共现的行排最前（一次检索可回答「部署端口」类问题）。
#[tokio::test]
async fn multi_word_query_ranks_cooccurrence_first() {
    let (_pool, svc, _pg) = setup().await;
    let p = svc
        .create_project("多词检索", "dev", None)
        .await
        .expect("建项目");
    svc.add_doc(
        p.id,
        "部署",
        "",
        "部署手册",
        "部署流程很长。\n部署端口是 17654。\n部署密钥管理。",
    )
    .await
    .unwrap();
    svc.add_doc(p.id, "部署", "", "别的话题", "这里没有部署也没有端口。")
        .await
        .unwrap();

    let hits = svc.search_doc_lines(p.id, "部署 端口", 50).await.unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits[0].text.contains("部署") && hits[0].text.contains("端口"),
        "两词共现行应排第一: {}",
        hits[0].text
    );
}

/// 验收②：归一化召回——「workbuddy」查「Work Buddy」（大小写/空白归一）。
#[tokio::test]
async fn normalized_name_is_recallable() {
    let (_pool, svc, _pg) = setup().await;
    let p = svc
        .create_project("归一化", "dev", None)
        .await
        .expect("建项目");
    svc.add_doc(
        p.id,
        "部署",
        "",
        "实体记录",
        "# 实体记录\n\nWork Buddy 是腾讯云桌面级 AI Agent 产品。\n",
    )
    .await
    .unwrap();

    for q in ["workbuddy", "WorkBuddy", "work buddy"] {
        let hits = svc.search_doc_lines(p.id, q, 50).await.unwrap();
        assert!(!hits.is_empty(), "query {q:?} 应召回 Work Buddy 行");
    }
}

/// 边界：空 query 报错；无命中返回空数组（不报错）。
#[tokio::test]
async fn empty_query_and_no_hit_boundaries() {
    let (_pool, svc, _pg) = setup().await;
    let p = svc
        .create_project("边界", "dev", None)
        .await
        .expect("建项目");
    svc.add_doc(p.id, "部署", "", "普通页", "普通内容。")
        .await
        .unwrap();

    let e = svc.search_doc_lines(p.id, "   ", 10).await.unwrap_err();
    assert!(e.to_string().contains("检索词不能为空"));
    let hits = svc
        .search_doc_lines(p.id, "不存在的词xyzzy", 10)
        .await
        .unwrap();
    assert!(hits.is_empty());
}
