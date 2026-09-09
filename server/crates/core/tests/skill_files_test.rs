//! 技能附属文件（folder 形态）：CRUD + 路径校验 + 导出包含文件。

use engram_core::skills::{SkillsError, SkillsService};
use support::{connect_with_retry, connection_url, start_pgvector};

mod support;

async fn svc() -> (SkillsService, support::TestPg) {
    let pg = start_pgvector().await.expect("测试库");
    let url = connection_url(&pg).await.expect("连接串");
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (SkillsService::new(pool), pg)
}

#[tokio::test]
async fn file_crud_roundtrip() {
    let (svc, _pg) = svc().await;
    svc.create_skill(engram_core::skills::NewSkill {
        slug: Some("review-pr"),
        name: "PR 审查",
        description: "审查拉取请求",
        content: "# 步骤",
        tags: &[],
        enabled: true,
        source: "manual",
        kind: "text",
        origin: "self",
        local_path: None,
        repo_url: None,
    })
    .await
    .expect("建技能");

    // 写两个文件
    let (path, size) = svc
        .put_file("review-pr", "scripts/check.txt", "print('hi')\n")
        .await
        .expect("put file");
    assert_eq!(path, "scripts/check.txt");
    assert!(size > 0);
    svc.put_file("review-pr", "references/api.md", "# API 参考")
        .await
        .expect("put file 2");

    // 索引（按 path 排序）
    let files = svc.list_files("review-pr").await.expect("index");
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["references/api.md", "scripts/check.txt"]);

    // 读回
    let content = svc
        .get_file("review-pr", "scripts/check.txt")
        .await
        .expect("get");
    assert_eq!(content, "print('hi')\n");

    // 覆盖更新（同路径幂等）
    svc.put_file("review-pr", "scripts/check.txt", "print('ok')")
        .await
        .expect("overwrite");
    let files = svc.list_files("review-pr").await.expect("index 2");
    assert_eq!(files.len(), 2);

    // 删除
    svc.delete_file("review-pr", "references/api.md")
        .await
        .expect("delete");
    let files = svc.list_files("review-pr").await.expect("index 3");
    assert_eq!(files.len(), 1);
}

#[tokio::test]
async fn path_validation_rejects_escape_and_entry() {
    let (svc, _pg) = svc().await;
    svc.create_skill(engram_core::skills::NewSkill {
        slug: Some("p-validation"),
        name: "校验",
        description: "",
        content: "",
        tags: &[],
        enabled: true,
        source: "manual",
        kind: "text",
        origin: "self",
        local_path: None,
        repo_url: None,
    })
    .await
    .expect("建技能");

    for bad in ["../escape.py", "/abs/path", "a//b", "a/./b", "SKILL.md", ""] {
        let err = svc
            .put_file("p-validation", bad, "x")
            .await
            .expect_err(&format!("应拒绝 {bad:?}"));
        assert!(matches!(err, SkillsError::BadRequest(_)), "{bad:?} → {err}");
    }
}

#[tokio::test]
async fn export_includes_files() {
    let (svc, _pg) = svc().await;
    svc.create_skill(engram_core::skills::NewSkill {
        slug: Some("with-files"),
        name: "带文件",
        description: "",
        content: "# 本体",
        tags: &[],
        enabled: true,
        source: "manual",
        kind: "text",
        origin: "self",
        local_path: None,
        repo_url: None,
    })
    .await
    .expect("建技能");
    svc.put_file("with-files", "scripts/run.txt", "echo hi")
        .await
        .expect("put file");

    let exported = svc.export_skills().await.expect("export");
    let entry = exported
        .iter()
        .find(|e| e.skill.slug == "with-files")
        .expect("entry");
    assert_eq!(entry.files.len(), 1);
    assert_eq!(entry.files[0].path, "scripts/run.txt");
    assert_eq!(entry.files[0].content, "echo hi");
}

#[tokio::test]
async fn bundle_export_roundtrip() {
    let (svc, _pg) = svc().await;
    svc.create_skill(engram_core::skills::NewSkill {
        slug: Some("bundle-me"),
        name: "打包技能",
        description: "整包导出",
        content: "# 本体正文",
        tags: &["ops".to_string()],
        enabled: true,
        source: "manual",
        kind: "text",
        origin: "self",
        local_path: None,
        repo_url: None,
    })
    .await
    .expect("建技能");
    svc.put_file("bundle-me", "scripts/run.txt", "echo hi")
        .await
        .expect("put file");

    let e = svc.export_one("bundle-me").await.expect("export_one");
    assert_eq!(e.files.len(), 1);
    assert_eq!(e.files[0].path, "scripts/run.txt");

    // SKILL.md 还原：import ↔ export 往返不丢元信息
    let md = engram_core::skills::render_skill_md(&e.skill);
    let (meta, body) = engram_core::skills::parse_frontmatter(&md);
    assert_eq!(meta.name.as_deref(), Some("打包技能"));
    assert_eq!(meta.description.as_deref(), Some("整包导出"));
    assert_eq!(meta.slug.as_deref(), Some("bundle-me"));
    assert_eq!(meta.tags, vec!["ops".to_string()]);
    assert_eq!(body.trim_start(), "# 本体正文");
}
