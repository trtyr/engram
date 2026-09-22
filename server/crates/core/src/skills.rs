//! 技能域服务：AI 技能（SKILL.md 形态）的资产化管理。
//!
//! 技能 = slug 唯一 + frontmatter（name/description/tags）+ markdown 正文的可复用指令包。
//! 语义字段每次变更前留版本快照（skill_revisions，保留最近 50 版），可回滚。
//! 批量导入直接吃 SKILL.md 全文（frontmatter 容错解析），迁移现有技能库零改写。
//!
//! 二态存储（0038）：text = 整体入库（无脚本，或依赖走 npm/cargo 全局二进制——全是文本）；
//! script = 真身只存本地文件夹（SKILL.md + scripts/ 等），库中只存指针（local_path）+ 来源
//! （origin: self/github/both，github 侧可记 repo_url）——content 不入库（get 现读、
//! 指针失效明确报错）、file_*/versions/restore 一律拒绝并指引本地操作、不产 revisions
//! 快照（版本归本地 git 管）。
//!
//! 持久化在 `engram_storage::repo::skills`（本文件只保留校验、冲突语义与编排；
//! 快照+变更的事务整体落在 repo 的 `*_tx` 函数内）。

mod crud;
mod files;
mod import_export;
mod model;
mod revisions;
pub use model::*;

use engram_storage::StoreError;
use engram_storage::repo::skills as repo;
use serde::Serialize;
use uuid::Uuid;

/// 技能域错误（api 层转 ApiError）。
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for SkillsError {
    fn from(e: StoreError) -> Self {
        SkillsError::Storage(e.to_string())
    }
}

/// 版本快照保留上限（防膨胀；更老的自动淘汰）。
pub use engram_storage::repo::skills::MAX_REVISIONS;

// ---------- 二态存储（0038）：text=入库 / script=本地指针 ----------

/// 合法存储形态。
pub const KINDS: &[&str] = &["text", "script"];

/// 合法来源：self=自建未发布 / github=源自 GitHub / both=自建且已发布。
pub const ORIGINS: &[&str] = &["self", "github", "both"];

/// 「真脚本」后缀清单：text 型技能的附属文件命中即拒绝（这类技能应整体走本地 + 指针）。
/// npm/cargo 全局二进制依赖只出现在 SKILL.md 说明文字里，不在附属文件，不受影响。
pub const SCRIPT_EXTS: &[&str] = &[
    "py", "sh", "bash", "zsh", "fish", "rb", "pl", "lua", "ps1", "bat", "cmd", "js", "mjs", "cjs",
    "ts",
];

/// 附属文件路径是否是「真脚本」（按后缀判定，大小写不敏感）。
pub fn is_script_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| SCRIPT_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

// ---------- frontmatter 容错解析 ----------

/// frontmatter 解析结果（全部可选——没有 frontmatter 也能导入，名字由调用方兜底）。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FrontmatterMeta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub slug: Option<String>,
    pub tags: Vec<String>,
}

// ---------- DTO（持久化模型在 storage，此处 re-export 保持路径兼容） ----------

pub use engram_storage::models::skills::{SkillDto, SkillRevisionDto, SkillSummaryDto};

// ---------- 附属文件（folder 形态） ----------

/// 单技能附属文件数上限。
pub const SKILL_FILES_MAX: usize = 64;

/// 附属文件索引条目（不含内容）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillFileInfoDto {
    /// 相对路径（/ 分隔，如 scripts/run.py）
    pub path: String,
    /// 内容字节数
    pub size: i64,
}

// ---------- Service ----------

pub struct SkillsService {
    pool: engram_storage::PgPool,
}

impl SkillsService {
    // ---------- 附属文件（folder 形态：scripts/ / references/ / assets/…） ----------
    //
    // skill = 文件夹：SKILL.md 本体在 content；附属文件按相对路径寻址。
    // 云部署语义：文件是「内容」不是「文件系统位置」——MCP 按路径下发，
    // AI 客户端取走后本地执行；服务端永不执行任何上传代码。
    //
    // 三种消费形态（按需取用，不一股脑拉全量）：
    // ① 纯文本 → skills_get 直接读（不落盘）；② 只要一个文件 →
    // GET /skills/{slug}/file?path=…&raw=1 单文件直下；③ 整个文件夹 →
    // GET /skills/{slug}/bundle（zip 整包）。
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_full() {
        let (meta, body) = parse_frontmatter(
            "---\nname: Review PR\ndescription: 审查拉取请求\nslug: review-pr\ntags: rust, review\n---\n正文第一行",
        );
        assert_eq!(meta.name.as_deref(), Some("Review PR"));
        assert_eq!(meta.description.as_deref(), Some("审查拉取请求"));
        assert_eq!(meta.slug.as_deref(), Some("review-pr"));
        assert_eq!(meta.tags, vec!["rust", "review"]);
        assert_eq!(body, "正文第一行");
    }

    #[test]
    fn frontmatter_crlf() {
        let (meta, body) = parse_frontmatter("---\r\nname: X\r\ntags: a, b\r\n---\r\nbody");
        assert_eq!(meta.name.as_deref(), Some("X"));
        assert_eq!(meta.tags, vec!["a", "b"]);
        assert_eq!(body, "body");
    }

    /// 现网 SKILL.md 真实形态：折叠块标量（>-）多行 description。
    #[test]
    fn frontmatter_folded_block_scalar() {
        let raw = "---\nname: chaitin-products\ndescription: >-\n  LOAD WHEN: 需要了解长亭产品线。\n  长亭产品知识库——包含产品白皮书、用户手册等。\n\n  TRIGGERS: 长亭, 万象, 雷池\n---\n\n# 长亭产品知识库\n\n## 用途\n";
        let (meta, body) = parse_frontmatter(raw);
        assert_eq!(meta.name.as_deref(), Some("chaitin-products"));
        let desc = meta.description.unwrap();
        assert!(desc.starts_with("LOAD WHEN: 需要了解长亭产品线。"));
        assert!(desc.contains("长亭产品知识库"));
        assert!(
            desc.ends_with("TRIGGERS: 长亭, 万象, 雷池"),
            "折叠标量多行并入一行：{desc}"
        );
        assert!(body.trim_start().starts_with("# 长亭产品知识库"));
    }

    /// 保留块标量（|）：换行保留。
    #[test]
    fn frontmatter_literal_block_scalar() {
        let (meta, _) =
            parse_frontmatter("---\nname: X\ndescription: |\n  第一行\n  第二行\n---\nbody");
        assert_eq!(meta.description.as_deref(), Some("第一行\n第二行"));
    }

    /// 块标量提前去缩进（下一键顶格）→ 块结束。
    #[test]
    fn frontmatter_block_scalar_ends_at_dedent() {
        let (meta, _) =
            parse_frontmatter("---\nname: X\ndescription: >-\n  折叠内容\ntags: a\n---\nb");
        assert_eq!(meta.description.as_deref(), Some("折叠内容"));
        assert_eq!(meta.tags, vec!["a"]);
    }

    #[test]
    fn frontmatter_unclosed_fence_treated_as_body() {
        let (meta, body) = parse_frontmatter("---\nname: X\nno_close");
        assert_eq!(meta.name, None);
        assert!(body.contains("name: X"));
    }

    #[test]
    fn frontmatter_closed_fence_without_trailing_newline() {
        let (meta, body) = parse_frontmatter("---\nname: X\n---");
        assert_eq!(meta.name.as_deref(), Some("X"));
        assert_eq!(body, "");
    }

    #[test]
    fn frontmatter_absent() {
        let (meta, body) = parse_frontmatter("# 直接正文");
        assert_eq!(meta, FrontmatterMeta::default());
        assert_eq!(body, "# 直接正文");
    }

    #[test]
    fn frontmatter_tags_block_list() {
        // D7：YAML 块列表 tags（tags: 后跟 "- item" 行）
        let (meta, body) = parse_frontmatter(
            "---
name: X
tags:
  - zztest
  - 标签二
---
正文",
        );
        assert_eq!(meta.tags, vec!["zztest", "标签二"]);
        assert_eq!(body, "正文");
    }

    #[test]
    fn body_strips_leading_blank_lines() {
        // 块标量/闭合行后的前导空行不应残留进正文
        let (_, body) = parse_frontmatter(
            "---
name: X
description: |
  多行
---


正文内容",
        );
        assert_eq!(body, "正文内容");
    }

    #[test]
    fn tags_json_style() {
        assert_eq!(parse_tags_value(r#"["a","b"]"#), vec!["a", "b"]);
        assert_eq!(parse_tags_value(" single "), vec!["single"]);
        assert_eq!(parse_tags_value(""), Vec::<String>::new());
    }

    #[test]
    fn slugify_basics() {
        assert_eq!(slugify("Review PR").as_deref(), Some("review-pr"));
        assert_eq!(
            slugify("  code_review.md ").as_deref(),
            Some("code-review-md")
        );
        assert_eq!(slugify("中文技能"), None);
        assert_eq!(slugify("mixed 中文 name").as_deref(), Some("mixed-name"));
    }

    #[test]
    fn slug_validation() {
        assert!(valid_slug("review-pr"));
        assert!(valid_slug("a"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("-lead"));
        assert!(!valid_slug("Has Upper"));
        assert!(!valid_slug("中文"));
        assert!(!valid_slug(&"a".repeat(81)));
    }

    #[test]
    fn script_ext_detection() {
        assert!(is_script_path("scripts/run.py"));
        assert!(is_script_path("run.sh"));
        assert!(is_script_path("a/b/c.PY")); // 大小写不敏感
        assert!(is_script_path("tools/x.ts"));
        assert!(is_script_path("x.mjs"));
        assert!(!is_script_path("references/api.md"));
        assert!(!is_script_path("README"));
        assert!(!is_script_path("data.json")); // json 不在清单（数据文件不是脚本）
    }

    #[test]
    fn two_kind_validation() {
        // script 缺 local_path
        assert!(SkillsService::validate_two_kind("script", "self", "", None, None).is_err());
        // script 带 content（正文不入库）
        assert!(
            SkillsService::validate_two_kind("script", "self", "正文", Some("/tmp/sk"), None)
                .is_err()
        );
        // text 带 local_path
        assert!(
            SkillsService::validate_two_kind("text", "self", "", Some("/tmp/sk"), None).is_err()
        );
        // origin=self 带 repo_url
        assert!(
            SkillsService::validate_two_kind(
                "text",
                "self",
                "",
                None,
                Some("https://github.com/a/b")
            )
            .is_err()
        );
        // 非法 kind / origin
        assert!(SkillsService::validate_two_kind("zip", "self", "", None, None).is_err());
        assert!(SkillsService::validate_two_kind("text", "mirror", "", None, None).is_err());
        // 合法 text（github 来源 + repo_url）
        let nk = SkillsService::validate_two_kind(
            "text",
            "github",
            "正文",
            None,
            Some("https://github.com/a/b"),
        )
        .unwrap();
        assert_eq!((nk.kind.as_str(), nk.origin.as_str()), ("text", "github"));
        assert_eq!(nk.local_path, None);
        assert_eq!(nk.repo_url.as_deref(), Some("https://github.com/a/b"));
        assert!(nk.snapshot);
        // 合法 script（trim 生效、不产快照）
        let nk = SkillsService::validate_two_kind(
            "script",
            "both",
            "",
            Some(" /tmp/my-skill "),
            Some(" https://github.com/a/b "),
        )
        .unwrap();
        assert_eq!((nk.kind.as_str(), nk.origin.as_str()), ("script", "both"));
        assert_eq!(nk.local_path.as_deref(), Some("/tmp/my-skill"));
        assert_eq!(nk.repo_url.as_deref(), Some("https://github.com/a/b"));
        assert!(!nk.snapshot);
    }
}
