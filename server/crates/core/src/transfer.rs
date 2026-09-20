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
    let wiki_libraries = repo::export_wiki_libraries(pool).await?;
    let wiki_pages = repo::export_wiki_pages(pool).await?;
    let (projects, locations, docs) = repo::export_projects(pool).await?;
    let todos = repo::export_todos(pool).await?;
    let kv_entries = repo::export_kv_entries(pool).await?;
    let wiki_promotions = repo::export_wiki_promotions(pool).await?;

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
            "skills": skills_json.len(), "wiki_libraries": wiki_libraries.len(),
    "wiki_pages": wiki_pages.len(),
            "projects": projects.len(), "locations": locations.len(), "docs": docs.len(),
            "todos": todos.len(), "kv_entries": kv_entries.len(),
            "wiki_promotions": wiki_promotions.len(),
        },
        "memory": {
            "sessions": sessions, "atoms": atoms, "scenarios": scenarios,
            "persona": persona, "entities": entities, "relations": relations,
        },
        "skills": skills_json,
        "wiki": { "libraries": wiki_libraries, "pages": wiki_pages },
        "projects": { "projects": projects, "locations": locations, "docs": docs },
        "todos": todos,
        "kv_entries": kv_entries,
        "wiki_promotions": wiki_promotions,
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

    let mem = import_memory_domain(pool, data).await?;
    let (c_skill, files_imported) = import_skills_domain(pool, data).await?;
    let (c_wikilib, c_wiki) = import_wiki_domain(pool, data).await?;
    let (c_project, c_location, c_doc) = import_projects_domain(pool, data).await?;
    let (t_imp, t_skip, kv_imp, kv_skip, p_imp, p_skip) = import_tail_domains(pool, data).await?;
    let (c_session, c_atom, c_scenario, c_persona, c_entity, c_relation) = (
        mem.sessions,
        mem.atoms,
        mem.scenarios,
        mem.persona,
        mem.entities,
        mem.relations,
    );

    Ok(json!({
        "format": "engram-transfer",
        "memory": {
            "sessions": c_session.to_json(), "atoms": c_atom.to_json(),
            "scenarios": c_scenario.to_json(), "persona": c_persona.to_json(),
            "entities": c_entity.to_json(), "relations": c_relation.to_json(),
        },
        "skills": { "skills": c_skill.to_json(), "files": { "imported": files_imported } },
        "wiki": { "libraries": c_wikilib.to_json(), "pages": c_wiki.to_json() },
        "projects": {
            "projects": c_project.to_json(), "locations": c_location.to_json(),
            "docs": c_doc.to_json(),
        },
        "todos": { "imported": t_imp, "skipped": t_skip },
        "kv_entries": { "imported": kv_imp, "skipped": kv_skip },
        "wiki_promotions": { "imported": p_imp, "skipped": p_skip },
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

// ---------- 双向同步转发（2026-09-18 数据同步线：CLI sync 的服务端形态） ----------
//
// 浏览器跨域无法直调目标实例，由本机服务端转发：push = 本地 export → POST 目标 import；
// pull = GET 目标 export → 灌本地 import。目标凭证用 migrate scope key（Bearer 直连），
// admin 密码不过目标网络。目标非 loopback 强制 https（allow_insecure 逃生）。

/// 校验并归一目标地址：非 loopback 且非 https 时拒绝（allow_insecure 逃生）。返回去尾斜杠地址。
pub fn check_sync_target(
    target: &str,
    allow_insecure: bool,
) -> std::result::Result<String, String> {
    let raw = target.trim().trim_end_matches('/');
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => return Err("target_url 必须含 scheme（http(s)://）".into()),
    };
    let host_end = rest.find(['/', ':']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
    if scheme != "https" && !loopback && !allow_insecure {
        return Err(format!(
            "目标 {raw} 非 loopback 且为 http 明文——公网同步必须走 TLS；ssh 隧道后可用 http://127.0.0.1:<port>；确要明文请传 allow_insecure=true"
        ));
    }
    Ok(raw.to_string())
}

/// 双向同步转发结果：源包分域计数 +（非 dry_run 时）目标导入报告。
pub struct SyncOutcome {
    pub direction: &'static str,
    pub source_counts: Value,
    pub dry_run: bool,
    pub import_report: Option<Value>,
}

/// 双向同步转发。dry_run=true 时 push 不连目标（仅本地导出对比）、pull 只拉不写。
pub async fn sync_transfer(
    pool: &PgPool,
    direction: &str,
    target_base: &str,
    token: &str,
    allow_insecure: bool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    if direction != "push" && direction != "pull" {
        return Err(TransferError::BadRequest(
            "direction 只支持 push/pull".into(),
        ));
    }
    let base = check_sync_target(target_base, allow_insecure).map_err(TransferError::BadRequest)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| TransferError::Storage(format!("HTTP 客户端构建失败: {e}")))?;

    match direction {
        "push" => {
            let bundle = export_bundle(pool).await?;
            let source_counts = bundle.get("counts").cloned().unwrap_or(Value::Null);
            if dry_run {
                return Ok(SyncOutcome {
                    direction: "push",
                    source_counts,
                    dry_run: true,
                    import_report: None,
                });
            }
            let resp = client
                .post(format!("{base}/migrate/import"))
                .bearer_auth(token)
                .json(&bundle)
                .send()
                .await
                .map_err(|e| TransferError::Storage(format!("目标不可达: {e}")))?;
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if !status.is_success() {
                return Err(TransferError::Storage(format!(
                    "目标导入失败（HTTP {status}）: {}",
                    serde_json::to_string(&body).unwrap_or_default()
                )));
            }
            Ok(SyncOutcome {
                direction: "push",
                source_counts,
                dry_run: false,
                import_report: Some(body),
            })
        }
        "pull" => {
            let resp = client
                .get(format!("{base}/migrate/export"))
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| TransferError::Storage(format!("目标不可达: {e}")))?;
            let status = resp.status();
            let bundle: Value = resp.json().await.unwrap_or(Value::Null);
            if !status.is_success() {
                return Err(TransferError::Storage(format!(
                    "目标导出失败（HTTP {status}）: {}",
                    serde_json::to_string(&bundle).unwrap_or_default()
                )));
            }
            let source_counts = bundle.get("counts").cloned().unwrap_or(Value::Null);
            if dry_run {
                return Ok(SyncOutcome {
                    direction: "pull",
                    source_counts,
                    dry_run: true,
                    import_report: None,
                });
            }
            let report = import_bundle(pool, &bundle).await?;
            Ok(SyncOutcome {
                direction: "pull",
                source_counts,
                dry_run: false,
                import_report: Some(report),
            })
        }
        _ => unreachable!("direction 已在入口校验"),
    }
}

/// memory 域导入计数（六个子域）。
struct MemoryImported {
    sessions: DomainCount,
    atoms: DomainCount,
    scenarios: DomainCount,
    persona: DomainCount,
    entities: DomainCount,
    relations: DomainCount,
}

/// memory 域导入：sessions → scenarios → atoms（外键序：scenarios 先于 atoms；自引用 FK 后置回填）
/// → persona → entities → relations。
async fn import_memory_domain(pool: &PgPool, data: &Value) -> Result<MemoryImported> {
    let mut c_session = DomainCount::default();
    for it in each(data.get("memory"), "sessions") {
        c_session.merge(DomainCount::bump(repo::import_session(pool, &it).await?));
    }
    // 外键序：scenarios 先于 atoms（atoms.scenario_id → scenarios）——顺序颠倒会在
    // 干净库导入时违反 atoms_scenario_id_fkey（公网加固 t9 往返演练抓出并修复）
    let mut c_scenario = DomainCount::default();
    for it in each(data.get("memory"), "scenarios") {
        c_scenario.merge(DomainCount::bump(repo::import_scenario(pool, &it).await?));
    }
    // superseded_by 是表内自引用 FK（atoms → atoms.id）——行序随机，插入期置 NULL，
    // 全量入库后统一回填（公网加固 t9 往返演练抓出）
    let mut c_atom = DomainCount::default();
    for it in each(data.get("memory"), "atoms") {
        let mut row = it.clone();
        if let Some(obj) = row.as_object_mut() {
            obj.insert("superseded_by".into(), serde_json::Value::Null);
        }
        c_atom.merge(DomainCount::bump(repo::import_atom(pool, &row).await?));
    }
    for it in each(data.get("memory"), "atoms") {
        let superseded_by = it
            .get("superseded_by")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if superseded_by.is_empty() {
            continue;
        }
        let (Ok(id), Ok(target)) = (
            uuid::Uuid::parse_str(it.get("id").and_then(|x| x.as_str()).unwrap_or("")),
            uuid::Uuid::parse_str(superseded_by),
        ) else {
            continue;
        };
        repo::backfill_atom_superseded_by(pool, id, target).await?;
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
    Ok(MemoryImported {
        sessions: c_session,
        atoms: c_atom,
        scenarios: c_scenario,
        persona: c_persona,
        entities: c_entity,
        relations: c_relation,
    })
}

/// 技能域导入（技能行 + 文件），返回 `(技能计数, 文件导入数)`。
async fn import_skills_domain(pool: &PgPool, data: &Value) -> Result<(DomainCount, usize)> {
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

    Ok((c_skill, files_imported))
}

/// wiki 域导入（库 + 页面，库先于页——页面外键指向库）。
async fn import_wiki_domain(pool: &PgPool, data: &Value) -> Result<(DomainCount, DomainCount)> {
    // wiki 多库迁移（2026-09-18 数据同步线补齐）：先导库行建「包内 library_id → 目标库 id」
    // 映射，页面按映射挂原库；旧包无 libraries 字段时页面 fallback main（v1 包兼容）。
    let mut lib_map: std::collections::HashMap<String, uuid::Uuid> =
        std::collections::HashMap::new();
    let mut c_wikilib = DomainCount::default();
    for it in each(data.get("wiki"), "libraries") {
        let src_id = it
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if src_id.is_empty() {
            continue;
        }
        let (mapped, imported) = repo::import_wiki_library(pool, &it).await?;
        lib_map.insert(src_id, mapped);
        c_wikilib.merge(DomainCount::bump(imported));
    }
    let main_lib = repo::main_library_id(pool).await?;
    let mut c_wiki = DomainCount::default();
    for it in each(data.get("wiki"), "pages") {
        // EN-63：导入页 tsv 按 wiki 口径现算（slug+title+content、wiki 分词变体）
        let tsv_text = engram_search::tokenize::tsv_text_wiki(&format!(
            "{} {} {}",
            it.get("slug").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("title").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("content").and_then(|x| x.as_str()).unwrap_or(""),
        ));
        let target_lib = it
            .get("library_id")
            .and_then(|x| x.as_str())
            .and_then(|s| lib_map.get(s).copied())
            .unwrap_or(main_lib);
        c_wiki.merge(DomainCount::bump(
            repo::import_wiki_page(pool, &it, &tsv_text, target_lib).await?,
        ));
    }
    Ok((c_wikilib, c_wiki))
}

/// 项目域导入（项目 → 位置 → 文档）。
async fn import_projects_domain(
    pool: &PgPool,
    data: &Value,
) -> Result<(DomainCount, DomainCount, DomainCount)> {
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
    Ok((c_project, c_location, c_doc))
}

/// todos / kv / promotions 三域导入（t9 补齐 v1 覆盖缺口）。
async fn import_tail_domains(
    pool: &PgPool,
    data: &Value,
) -> Result<(usize, usize, usize, usize, usize, usize)> {
    // todos / kv / promotions 域（t9 补齐 v1 覆盖缺口；promotions 的 library 映射目标 main 库）
    let todo_items = data
        .get("todos")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let (t_imp, t_skip) = repo::import_todos(pool, &todo_items).await?;

    let kv_items = data
        .get("kv_entries")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let (kv_imp, kv_skip) = repo::import_kv_entries(pool, &kv_items).await?;

    let promo_items = data
        .get("wiki_promotions")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let (p_imp, p_skip) = repo::import_wiki_promotions(pool, &promo_items).await?;
    Ok((t_imp, t_skip, kv_imp, kv_skip, p_imp, p_skip))
}
