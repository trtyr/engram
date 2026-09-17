//! 迁移编排：全系统导出聚合、导入（冲突跳过，分域报告）、技能导入闭环。
//!
//! 迁移包格式（engram-transfer v1）：
//! `{format, version, exported_at, counts, memory:{sessions,atoms,scenarios,persona,entities,relations},
//!   skills:[{skill...,files:[...]}], wiki:{pages:[...]}, projects:{projects,locations,docs}}`
//!
//! 导入语义：冲突跳过（主键/slug/name 已存在即 skip）——迁移（空机全量进）与
//! 合并（两机并集）都可重复执行。派生列（embedding/tsv）不迁移，导入时重建
//! tsv；向量空缺用 memory reembed / wiki re-embed 补。codegraph 索引是派生数据
//! 不迁移（B 机对同源仓库重新建索引即可）。

use engram_storage::repo::transfer as repo;
use engram_storage::{PgPool, StoreError};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for TransferError {
    fn from(e: StoreError) -> Self {
        TransferError::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TransferError>;

/// 全系统导出聚合（迁移包 v1）。
pub async fn export_bundle(pool: &PgPool) -> Result<Value> {
    let (sessions, atoms, scenarios, persona, entities, relations) =
        repo::export_memory(pool).await?;
    let skills = repo::export_skills_with_files(pool).await?;
    let wiki_pages = repo::export_wiki_pages(pool).await?;
    let (projects, locations, docs) = repo::export_projects(pool).await?;

    let skills_json: Vec<Value> = skills
        .into_iter()
        .map(|(mut skill, files)| {
            skill["files"] = json!(files);
            skill
        })
        .collect();

    Ok(json!({
        "format": "engram-transfer",
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "counts": {
            "sessions": sessions.len(), "atoms": atoms.len(),
            "scenarios": scenarios.len(), "persona": persona.len(),
            "entities": entities.len(), "relations": relations.len(),
            "skills": skills_json.len(), "wiki_pages": wiki_pages.len(),
            "projects": projects.len(), "locations": locations.len(), "docs": docs.len(),
        },
        "memory": {
            "sessions": sessions, "atoms": atoms, "scenarios": scenarios,
            "persona": persona, "entities": entities, "relations": relations,
        },
        "skills": skills_json,
        "wiki": { "pages": wiki_pages },
        "projects": { "projects": projects, "locations": locations, "docs": docs },
    }))
}

/// 分域导入计数（imported, skipped）。
#[derive(Default)]
struct DomainCount {
    imported: usize,
    skipped: usize,
}

impl DomainCount {
    fn bump(imported: bool) -> Self {
        if imported {
            DomainCount {
                imported: 1,
                skipped: 0,
            }
        } else {
            DomainCount {
                imported: 0,
                skipped: 1,
            }
        }
    }
    fn merge(&mut self, other: DomainCount) {
        self.imported += other.imported;
        self.skipped += other.skipped;
    }
    fn to_json(&self) -> Value {
        json!({ "imported": self.imported, "skipped": self.skipped })
    }
}

fn each(v: Option<&Value>, key: &str) -> Vec<Value> {
    v.and_then(|d| d.get(key))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 导入迁移包（冲突跳过）。返回分域 {imported, skipped} 报告。
pub async fn import_bundle(pool: &PgPool, data: &Value) -> Result<Value> {
    if data.get("format").and_then(|x| x.as_str()) != Some("engram-transfer") {
        return Err(TransferError::BadRequest(
            "不是 engram-transfer 迁移包——请用 /migrate/export 的导出文件".into(),
        ));
    }

    let mut c_session = DomainCount::default();
    for it in each(data.get("memory"), "sessions") {
        c_session.merge(DomainCount::bump(repo::import_session(pool, &it).await?));
    }
    let mut c_atom = DomainCount::default();
    for it in each(data.get("memory"), "atoms") {
        c_atom.merge(DomainCount::bump(repo::import_atom(pool, &it).await?));
    }
    let mut c_scenario = DomainCount::default();
    for it in each(data.get("memory"), "scenarios") {
        c_scenario.merge(DomainCount::bump(repo::import_scenario(pool, &it).await?));
    }
    let mut c_persona = DomainCount::default();
    for it in each(data.get("memory"), "persona") {
        c_persona.merge(DomainCount::bump(repo::import_persona(pool, &it).await?));
    }
    let mut c_entity = DomainCount::default();
    for it in each(data.get("memory"), "entities") {
        c_entity.merge(DomainCount::bump(repo::import_entity(pool, &it).await?));
    }
    let mut c_relation = DomainCount::default();
    for it in each(data.get("memory"), "relations") {
        c_relation.merge(DomainCount::bump(
            repo::import_entity_relation(pool, &it).await?,
        ));
    }

    let mut c_skill = DomainCount::default();
    let mut files_imported = 0usize;
    for s in each(Some(data), "skills") {
        let files = s
            .get("files")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let (imported, files_n) = repo::import_skill(pool, &s, &files).await?;
        c_skill.merge(DomainCount::bump(imported));
        files_imported += files_n;
    }

    let mut c_wiki = DomainCount::default();
    for it in each(data.get("wiki"), "pages") {
        // EN-63：导入页 tsv 按 wiki 口径现算（slug+title+content、wiki 分词变体）
        let tsv_text = engram_search::tokenize::tsv_text_wiki(&format!(
            "{} {} {}",
            it.get("slug").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("title").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("content").and_then(|x| x.as_str()).unwrap_or(""),
        ));
        c_wiki.merge(DomainCount::bump(repo::import_wiki_page(pool, &it, &tsv_text).await?));
    }

    let mut c_project = DomainCount::default();
    for it in each(data.get("projects"), "projects") {
        c_project.merge(DomainCount::bump(repo::import_project(pool, &it).await?));
    }
    let mut c_location = DomainCount::default();
    for it in each(data.get("projects"), "locations") {
        c_location.merge(DomainCount::bump(
            repo::import_project_location(pool, &it).await?,
        ));
    }
    let mut c_doc = DomainCount::default();
    for it in each(data.get("projects"), "docs") {
        c_doc.merge(DomainCount::bump(
            repo::import_project_doc(pool, &it).await?,
        ));
    }

    Ok(json!({
        "format": "engram-transfer",
        "memory": {
            "sessions": c_session.to_json(), "atoms": c_atom.to_json(),
            "scenarios": c_scenario.to_json(), "persona": c_persona.to_json(),
            "entities": c_entity.to_json(), "relations": c_relation.to_json(),
        },
        "skills": { "skills": c_skill.to_json(), "files": { "imported": files_imported } },
        "wiki": { "pages": c_wiki.to_json() },
        "projects": {
            "projects": c_project.to_json(), "locations": c_location.to_json(),
            "docs": c_doc.to_json(),
        },
        "note": "冲突（id/slug/name 已存在）按跳过处理；embedding 未迁移——\
                 memory 用 POST /memory/reembed、wiki 用文档域 re-embed 补齐",
    }))
}

/// 技能域导入闭环：吃 /skills/export（或迁移包 skills 数组）同构数据。
pub async fn import_skills_bundle(pool: &PgPool, items: &Value) -> Result<Value> {
    let list = items
        .as_array()
        .cloned()
        .or_else(|| items.get("skills").and_then(|x| x.as_array()).cloned())
        .ok_or_else(|| {
            TransferError::BadRequest(
                "格式不对：期望数组（/skills/export 的返回）或含 skills 字段的对象".into(),
            )
        })?;
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut files = 0usize;
    for s in list {
        let fs = s
            .get("files")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let (ok, fn_) = repo::import_skill(pool, &s, &fs).await?;
        if ok {
            imported += 1;
            files += fn_;
        } else {
            skipped += 1;
        }
    }
    Ok(json!({
        "imported": imported, "skipped": skipped, "files_imported": files,
        "note": "slug 已存在按跳过处理（不覆盖）——更新请走 skills_update",
    }))
}

// ---------- 远程拉取（A → B 一键迁移） ----------
//
// B 机管理员提供 A 机地址 + A 的 admin 密码：登录 A → 拉迁移包 → 落地本机。
// A 的密码只在本次请求内使用（登录换 ams_ token），不落库不缓存。

/// 从远端 engram 拉取迁移包并导入本机。返回导入报告（含来源与包 counts）。
pub async fn pull_from(
    pool: &PgPool,
    source_base: &str,
    source_admin_password: &str,
) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| TransferError::Storage(format!("HTTP 客户端构建失败: {e}")))?;

    // 1) 登录 A（拿 ams_ 会话）
    let login: Value = client
        .post(format!("{source_base}/auth/login"))
        .json(&serde_json::json!({ "password": source_admin_password }))
        .send()
        .await
        .map_err(|e| TransferError::BadRequest(format!("连接源机失败（{source_base}）: {e}")))?
        .error_for_status()
        .map_err(|e| {
            TransferError::BadRequest(match e.status() {
                Some(code) if code.as_u16() == 401 => "源机认证失败：admin 密码错误".into(),
                Some(code) => format!("源机登录失败：HTTP {code}"),
                _ => e.to_string(),
            })
        })?
        .json()
        .await
        .map_err(|e| TransferError::BadRequest(format!("源机登录响应解析失败: {e}")))?;
    let token = login
        .get("token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| TransferError::BadRequest("源机登录响应缺 token——确认对端是 engram".into()))?
        .to_string();

    // 2) 拉迁移包
    let bundle = client
        .get(format!("{source_base}/migrate/export"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| TransferError::BadRequest(format!("拉取迁移包失败: {e}")))?
        .error_for_status()
        .map_err(|e| {
            TransferError::BadRequest(format!(
                "源机导出失败：HTTP {}",
                e.status().map(|c| c.as_u16()).unwrap_or(0)
            ))
        })?
        .json::<Value>()
        .await
        .map_err(|e| TransferError::BadRequest(format!("迁移包解析失败: {e}")))?;

    let counts = bundle.get("counts").cloned().unwrap_or(json!({}));

    // 3) 落地本机
    let report = import_bundle(pool, &bundle).await?;

    Ok(json!({
        "source": source_base,
        "bundle_counts": counts,
        "imported": report,
    }))
}
