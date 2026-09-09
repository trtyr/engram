//! 二态存储（0038）：text / script 两种形态的域级行为。
//!
//! 覆盖：text 型附属脚本文件被拒、script 型缺 local_path / 带 content 被拒、
//! 指针现读与指针失效、file_*/versions/restore 对 script 型拒绝、script 不产快照、
//! text 型快照行为不回归。

use engram_core::skills::{NewSkill, SkillsError, SkillsService};
use support::{connect_with_retry, connection_url, start_pgvector};

mod support;

async fn svc() -> (SkillsService, support::TestPg) {
    let pg = start_pgvector().await.expect("测试库");
    let url = connection_url(&pg).await.expect("连接串");
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (SkillsService::new(pool), pg)
}

/// 建一个本地技能文件夹（temp 下唯一目录），写入 SKILL.md，返回路径。
fn make_local_skill_dir(name: &str, skill_md: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "engram-two-kind-{}-{name}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("SKILL.md"), skill_md).expect("write SKILL.md");
    dir
}

#[tokio::test]
async fn text_rejects_script_attachment() {
    let (svc, _pg) = svc().await;
    svc.create_skill(NewSkill {
        slug: Some("text-only"),
        name: "纯文本",
        description: "",
        content: "# 纯文本技能",
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

    for script in ["scripts/run.py", "run.sh", "x.JS"] {
        let err = svc
            .put_file("text-only", script, "echo hi")
            .await
            .expect_err(&format!("应拒绝脚本附件 {script:?}"));
        let msg = err.to_string();
        assert!(msg.contains("script"), "{script:?} → {msg}");
    }
    // 非脚本附件照常
    svc.put_file("text-only", "references/api.md", "# api")
        .await
        .expect("文本附件应放行");
}

#[tokio::test]
async fn script_create_validation() {
    let (svc, _pg) = svc().await;
    // 缺 local_path
    let err = svc
        .create_skill(NewSkill {
            slug: Some("no-path"),
            name: "缺指针",
            description: "",
            content: "",
            tags: &[],
            enabled: true,
            source: "manual",
            kind: "script",
            origin: "self",
            local_path: None,
            repo_url: None,
        })
        .await
        .expect_err("缺 local_path 应拒");
    assert!(err.to_string().contains("local_path"));
    // 带 content（正文不入库）
    let err = svc
        .create_skill(NewSkill {
            slug: Some("with-content"),
            name: "带正文",
            description: "",
            content: "# 不该入库",
            tags: &[],
            enabled: true,
            source: "manual",
            kind: "script",
            origin: "self",
            local_path: Some("/tmp/whatever"),
            repo_url: None,
        })
        .await
        .expect_err("script 带 content 应拒");
    assert!(err.to_string().contains("正文"));
}

#[tokio::test]
async fn script_pointer_lifecycle() {
    let (svc, _pg) = svc().await;
    let dir = make_local_skill_dir("lifecycle", "# 本地真身\n\n由指针现读。");
    let dir_str = dir.to_string_lossy().to_string();

    let created = svc
        .create_skill(NewSkill {
            slug: Some("local-tool"),
            name: "本地工具",
            description: "script 型全生命周期",
            content: "",
            tags: &[],
            enabled: true,
            source: "manual",
            kind: "script",
            origin: "both",
            local_path: Some(&dir_str),
            repo_url: Some("https://github.com/x/local-tool"),
        })
        .await
        .expect("建 script 技能");
    assert_eq!(created.kind, "script");
    assert_eq!(created.local_path.as_deref(), Some(dir_str.as_str()));
    assert!(created.content.is_empty(), "正文不入库");

    // 纯库读：content 恒空
    let raw = svc.get_skill("local-tool").await.expect("库读");
    assert!(raw.content.is_empty());

    // 现读组装：content == 本地 SKILL.md
    let resolved = svc
        .get_skill_with_content("local-tool")
        .await
        .expect("现读");
    assert!(resolved.content.contains("本地真身"));

    // 指针失效：目录被移走 → 明确报错
    let moved = dir.with_extension("moved");
    std::fs::rename(&dir, &moved).expect("rename");
    let err = svc
        .get_skill_with_content("local-tool")
        .await
        .expect_err("指针应失效");
    assert!(matches!(err, SkillsError::NotFound(_)), "{err}");
    assert!(err.to_string().contains("指针失效"), "{err}");
    std::fs::rename(&moved, &dir).expect("rename back");

    // file_* 全拒
    for (msg, r) in [
        ("files", svc.list_files("local-tool").await.err()),
        (
            "get_file",
            svc.get_file("local-tool", "scripts/a.txt").await.err(),
        ),
        (
            "put_file",
            svc.put_file("local-tool", "scripts/a.txt", "x").await.err(),
        ),
        (
            "delete_file",
            svc.delete_file("local-tool", "scripts/a.txt").await.err(),
        ),
    ] {
        let err = r.unwrap_or_else(|| panic!("{msg} 应拒绝"));
        let m = err.to_string();
        assert!(m.contains("本地"), "{msg} → {m}");
    }

    // versions/restore 拒（script 不产快照）
    let err = svc
        .list_revisions("local-tool")
        .await
        .expect_err("versions 应拒绝");
    assert!(matches!(err, SkillsError::BadRequest(_)), "{err}");
    let err = svc
        .restore_revision("local-tool", uuid::Uuid::now_v7())
        .await
        .expect_err("restore 应拒绝");
    assert!(matches!(err, SkillsError::BadRequest(_)), "{err}");

    // update 正文拒绝；改元数据 OK 且不留快照（无内容可回滚）
    let err = svc
        .update_skill(
            "local-tool",
            engram_core::skills::SkillPatch {
                content: Some("# 新正文".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("script 改正文应拒");
    assert!(err.to_string().contains("本地"), "{err}");
    svc.update_skill(
        "local-tool",
        engram_core::skills::SkillPatch {
            description: Some("改描述".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("script 改描述应放行");

    // 指针搬家（update local_path）
    let dir2 = make_local_skill_dir("lifecycle2", "# 搬家后的真身");
    svc.update_skill(
        "local-tool",
        engram_core::skills::SkillPatch {
            local_path: Some(dir2.to_string_lossy().to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("script 改 local_path 应放行");
    let resolved = svc
        .get_skill_with_content("local-tool")
        .await
        .expect("新指针现读");
    assert!(resolved.content.contains("搬家后的真身"));

    // 导出：只带元数据（content 空、无 files）
    let e = svc.export_one("local-tool").await.expect("导出");
    assert!(e.skill.content.is_empty());
    assert!(e.files.is_empty());

    let _ = std::fs::remove_dir_all(&dir2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn text_snapshot_behavior_unchanged() {
    let (svc, _pg) = svc().await;
    svc.create_skill(NewSkill {
        slug: Some("still-text"),
        name: "文本照旧",
        description: "",
        content: "# v1",
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
    // create 快照仍在
    assert_eq!(
        svc.list_revisions("still-text").await.expect("revs").len(),
        1
    );
    // update 留快照、回滚可用
    svc.update_skill(
        "still-text",
        engram_core::skills::SkillPatch {
            content: Some("# v2".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(
        svc.list_revisions("still-text").await.expect("revs").len(),
        2
    );
    // 列表摘要带二态字段
    let list = svc.list_skills(None, None, None).await.expect("list");
    let s = list.iter().find(|s| s.slug == "still-text").expect("entry");
    assert_eq!(s.kind, "text");
    assert_eq!(s.origin, "self");
    assert!(s.local_path.is_none());
}
