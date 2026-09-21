//! `bridge` 的实现切片（架构治理 2026-09-21：自 bridge.rs 纯搬移，零行为变化）。

use super::*;

/// 注册 CodeGraph 域 job handlers（main 装配用）：cg_index / cg_sync。
/// payload: {"project_id": uuid}。一切长操作走队列——索引不再阻塞 HTTP 10 分钟。
pub fn register_handlers(
    runner: engram_jobs::Runner,
    bridge_root: std::path::PathBuf,
) -> engram_jobs::Runner {
    let root_index = bridge_root.clone();
    runner
        .register("cg_index", move |ctx| {
            let root = root_index.clone();
            async move {
                let id: Uuid = ctx
                    .job
                    .payload
                    .0
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| {
                        engram_jobs::types::JobError::Permanent("payload 缺 project_id".into())
                    })?;
                let bridge = CgBridge::new(ctx.pool().clone(), root);
                let dto = bridge.index(id).await.map_err(classify)?;
                Ok(serde_json::json!({"project_id": id, "status": dto.status}))
            }
        })
        .register("cg_sync", move |ctx| {
            let root = bridge_root.clone();
            async move {
                let id: Uuid = ctx
                    .job
                    .payload
                    .0
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| {
                        engram_jobs::types::JobError::Permanent("payload 缺 project_id".into())
                    })?;
                let bridge = CgBridge::new(ctx.pool().clone(), root);
                let dto = bridge.sync(id).await.map_err(classify)?;
                Ok(serde_json::json!({"project_id": id, "status": dto.status}))
            }
        })
}

impl CgBridge {
    /// 注册项目（入口收敛 2026-09-21）：**只接受 git 仓库地址**——本地路径注册已退场（既有行不动）。
    ///
    /// 落盘：`dest_parent` 留空 → `<codegraph 根>/<项目名>/`（服务端自建，删条目连目录清）；
    /// 给了 → `<父目录>/<仓库名>/`（用户指定，删条目只删注册与产物、目录保留；父目录须白名单内）。
    /// clone 用 `git clone --depth 1`（超时 `CLONE_TIMEOUT_SECS`）；**失败不落条目并清理半成品目录**。
    pub async fn register(
        &self,
        name: &str,
        source_uri: &str,
        dest_parent: Option<&str>,
    ) -> Result<CgProjectDto, CgError> {
        if !looks_like_git_uri(source_uri) {
            return Err(CgError::BadRequest(format!(
                "只接受 git 仓库地址（如 https://github.com/you/repo）：收到 {source_uri:?}。\
                 本机已有源码想建图 → 用「上传产物」入口：本机 `codegraph index` 后把 \
                 .codegraph/codegraph.db 传上来（服务端不再接受本地路径——云端看不到客户端文件系统）"
            )));
        }
        let existing: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM cg_projects WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        if existing.is_some() {
            return Err(CgError::BadRequest(format!("项目名 {name} 已存在")));
        }
        // 同一仓库只许注册一次——避免同库多份索引
        let dup: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id, name FROM cg_projects WHERE source_uri = $1")
                .bind(source_uri)
                .fetch_optional(&self.pool)
                .await?;
        if let Some((_, holder)) = dup {
            return Err(CgError::BadRequest(format!(
                "该来源已注册为项目 {holder}——同源一个索引，直接复用即可"
            )));
        }

        // 落盘位置：默认（服务端自建）或自定义（用户父目录 + 仓库名）
        let repo_name = repo_name_from_uri(source_uri).unwrap_or_else(|| sanitize_dir_name(name));
        let (target, dest_mode) = match dest_parent.map(str::trim).filter(|s| !s.is_empty()) {
            Some(parent) => (custom_dest(&self.root, parent, &repo_name)?, "custom"),
            None => (default_dest(&self.root, name), "default"),
        };
        if let Some(p) = target.parent()
            && let Err(e) = tokio::fs::create_dir_all(p).await
        {
            return Err(CgError::BadRequest(format!(
                "落盘目录不可创建（{}）：{e}——检查服务端磁盘与权限",
                p.display()
            )));
        }

        // clone（超时保护 + 失败清理：绝不留下半成品目录，也绝不落条目）
        let cloned = tokio::time::timeout(
            Duration::from_secs(CLONE_TIMEOUT_SECS),
            tokio::process::Command::new("git")
                .args(["clone", "--depth", "1", source_uri])
                .arg(&target)
                .output(),
        )
        .await;
        let out = match cloned {
            Err(_) => {
                let _ = tokio::fs::remove_dir_all(&target).await;
                return Err(CgError::BadRequest(format!(
                    "clone 超时（{CLONE_TIMEOUT_SECS}s）：{source_uri}——网络不可达或仓库过大"
                )));
            }
            Ok(Err(e)) => {
                let _ = tokio::fs::remove_dir_all(&target).await;
                return Err(CgError::BadRequest(format!(
                    "git 不可用（服务端需装 git 且在 PATH）：{e}"
                )));
            }
            Ok(Ok(o)) => o,
        };
        if !out.status.success() {
            let _ = tokio::fs::remove_dir_all(&target).await;
            return Err(CgError::BadRequest(format!(
                "clone 失败（{source_uri}）：{}——检查仓库地址/网络/访问权限（私有仓库需服务端有凭证）",
                truncate(&String::from_utf8_lossy(&out.stderr), 300)
            )));
        }

        let path = target.to_string_lossy().into_owned();
        let row = sqlx::query_as::<_, CgProjectDto>(
            "INSERT INTO cg_projects (id, name, path, source_uri, status, dest_mode) \
             VALUES ($1, $2, $3, $4, 'registered', $5) RETURNING *",
        )
        .bind(Uuid::now_v7())
        .bind(name)
        .bind(&path)
        .bind(source_uri)
        .bind(dest_mode)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// 产物上传（公网多Agent P001 步骤4；codegraph 上云 task-2 起为「投递」语义）：
    /// 客户端本机 `codegraph CLI index` 后，上传 `.codegraph/codegraph.db` + HEAD。
    /// 服务端只存产物 + 声明式新鲜度，**无代码、无 git 凭证**；CLI 是基础设施（CG_VERSION_PIN）。
    ///
    /// **两入口都可刷新同一记录**（README《核心模型》）：name 不存在则新建条目（client_upload）；
    /// 已存在则不论当前 `source_kind`（cloud_index / client_upload）都覆盖当前产物——被覆盖那份的
    /// 元数据压进 `stats.previous` 留痕。0051 的旧规则（本机索引不归上传通道管）只在「服务端自己管着那个 path」的
    /// 本地实例成立，云端不成立，故撤销（R1「后到者覆盖 + 完整留痕」）。
    pub async fn upload_artifact(
        &self,
        name: &str,
        head: &str,
        producer: &str,
        db_bytes: &[u8],
    ) -> Result<CgProjectDto, CgError> {
        // 校验：head 是 commit hash（7~40 位 hex，短/长 SHA 都收）；**空 = 未声明**
        // （2026-09-21 入口收敛：Web 上传入口只有文件选择器，拿不到 commit hash——
        //  未声明时落 NULL 且新鲜度如实标注「无法比对」，既不编造 head 也不因缺它拒收）。
        let head = head.trim();
        if !head.is_empty()
            && (!(7..=40).contains(&head.len()) || !head.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(CgError::BadRequest(format!(
                "head 不是合法 commit hash（7~40 位 hex，收到 {} 字符）——客户端本机 `git rev-parse HEAD` 取，\
                 留空表示未声明",
                head.len()
            )));
        }
        // 声明式来源与 head 落库口径（2026-09-21 入口收敛）：有 head → `upload://<head>` + head 列；
        // 未声明（Web 入口只能选文件）→ `upload://(undeclared)` + NULL，新鲜度如实标注「无法比对」。
        let source_uri = if head.is_empty() {
            "upload://(undeclared)".to_string()
        } else {
            format!("upload://{head}")
        };
        let head_opt: Option<&str> = if head.is_empty() { None } else { Some(head) };

        // 校验：SQLite 魔数（坏产物当场拒——「能存进去但查不了」才是最差的体验）
        const SQLITE_MAGIC: &[u8] = b"SQLite format 3\x00";
        if db_bytes.len() < SQLITE_MAGIC.len() || &db_bytes[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
            return Err(CgError::BadRequest(
                "db 不是 SQLite 文件（缺「SQLite format 3」魔数）——请上传 codegraph CLI 产出的 \
             .codegraph/codegraph.db 本体（原始二进制，不要压缩/文本化）"
                    .into(),
            ));
        }
        // 上限 256MB（R4 已定 v1 整库上传；超限的正解是改走云端自建入口）
        if db_bytes.len() > 256 * 1024 * 1024 {
            return Err(CgError::BadRequest(format!(
                "db 超限（{} MB > 256 MB）——改用云端自建入口（register + index）让它拉整仓库索引，\
                 或拆分仓库后重传",
                db_bytes.len() / 1024 / 1024
            )));
        }

        // 当前记录（可能已有产物）：用于「后到者覆盖 + 上一份留痕」
        let prev: Option<CgProjectDto> =
            sqlx::query_as::<_, CgProjectDto>("SELECT * FROM cg_projects WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;

        let id = match &prev {
            Some(p) => p.id,
            None => {
                let nid = Uuid::now_v7();
                let dir = self
                    .root
                    .join("uploads")
                    .join(nid.to_string())
                    .join(".codegraph");
                sqlx::query_as::<_, CgProjectDto>(
                    "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind) \
                 VALUES ($1, $2, $3, $4, 'ready', 'client_upload') RETURNING *",
                )
                .bind(nid)
                .bind(name)
                .bind(
                    dir.parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
                .bind(&source_uri)
                .fetch_one(&self.pool)
                .await?;
                nid
            }
        };

        // 原子落盘：temp + rename（半写的 db 不该被查询看到）
        let dir = self
            .root
            .join("uploads")
            .join(id.to_string())
            .join(".codegraph");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;
        let tmp = dir.join("codegraph.db.tmp");
        let dst = dir.join("codegraph.db");
        tokio::fs::write(&tmp, db_bytes)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;

        // R3 产物版本校验（2026-09-21 已定口径：CLI 版本**硬拒** / extraction 版本仅告警）。
        // 在 rename 之前把关——不合规的产物不落正式路径（宁缺勿脏：静默脏数据比拒收难救得多）。
        let (built_with, extraction_warning) = match Self::read_artifact_versions(&tmp) {
            Ok((Some(v), ext)) => {
                if v != CG_VERSION_PIN {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(CgError::BadRequest(format!(
                        "产物版本不符：db 内 indexed_with_version={v}，本服务 pin={CG_VERSION_PIN}——\
                         请客户端把 CLI 对齐到 {CG_VERSION_PIN} 后重新索引再传：\
                         `codegraph upgrade {CG_VERSION_PIN}` → `codegraph index` → 重传。\
                         （版本不符的产物 schema 可能不兼容，服务端不接收）"
                    )));
                }
                let warn = ext.filter(|e| e != CG_EXTRACTION_VERSION_SEEN).map(|e| {
                    format!(
                        "extraction 版本 {e} ≠ 本服务实测口径 {CG_EXTRACTION_VERSION_SEEN}——\
                         图语义可能略有差异（已入库，未阻断）"
                    )
                });
                (Some(v), warn)
            }
            Ok((None, _)) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(CgError::BadRequest(
                    "产物缺少 project_metadata.indexed_with_version——不是 codegraph CLI 产出的\
                     索引库？请上传 `.codegraph/codegraph.db` 本体（原始二进制）"
                        .into(),
                ));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(e);
            }
        };

        tokio::fs::rename(&tmp, &dst)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;

        // 产物元数据（迁移 0055）：来源标注改判 client_upload、产物落点随之指向上传工作根、
        // head/built_with_version/produced_at/last_producer 落库；被覆盖那一条压进 stats.previous 留痕。
        let path = dir
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut stats =
            with_previous_trace(prev.as_ref().and_then(|p| p.stats.clone()), prev.as_ref());
        if let Some(w) = extraction_warning {
            let obj = stats.get_or_insert_with(|| serde_json::json!({}));
            if let Some(m) = obj.as_object_mut() {
                m.insert("extraction_warning".to_string(), serde_json::json!(w));
            }
        }
        sqlx::query(
            "UPDATE cg_projects SET source_kind = 'client_upload', path = $2, source_uri = $3, \
         head = $4, uploaded_at = now(), produced_at = now(), built_with_version = $5, \
         last_producer = $6, stats = $7, status = 'ready', error = NULL, last_synced_at = now(), \
         updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(&path)
        .bind(&source_uri)
        .bind(head_opt)
        .bind(&built_with)
        .bind(producer)
        .bind(stats)
        .execute(&self.pool)
        .await?;
        self.get(id).await
    }

    /// 读产物 db 内的版本声明（R3）：`project_metadata` 是 key/value 表
    /// （`key`/`value`/`updated_at`；值如 `indexed_with_version=1.5.0`、
    /// `indexed_with_extraction_version=24`）。
    ///
    /// 缺表 / 缺键 → `None`（调用方按「无法校验」处理，见 `upload_artifact`）。
    fn read_artifact_versions(db_path: &Path) -> Result<(Option<String>, Option<String>), CgError> {
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| {
            CgError::BadRequest(format!(
                "产物 db 打不开（不是可读的 SQLite 库？）: {e}——请上传 codegraph CLI 产出的 \
                 .codegraph/codegraph.db 本体（原始二进制，不要压缩/文本化）"
            ))
        })?;
        let read = |key: &str| -> Option<String> {
            conn.query_row(
                "SELECT value FROM project_metadata WHERE key = ?1",
                rusqlite::params![key],
                |r| r.get::<_, String>(0),
            )
            .ok()
        };
        Ok((
            read("indexed_with_version"),
            read("indexed_with_extraction_version"),
        ))
    }

    /// 云自建路径的产物元数据回写（迁移 0055 三件套）：产出时刻 + CLI 版本 + 投递者。
    /// 若该记录此前是 client_upload（上一份产物来自客户端），上一份压进 `stats.previous` 留痕
    /// ——与 `upload_artifact` 同口径（R1「后到者覆盖 + 完整留痕」）。
    pub async fn mark_cloud_artifact(&self, id: Uuid) -> Result<(), CgError> {
        let cur = self.get(id).await?;
        let ver = self.cli_status().await.version;
        let prev = if cur.source_kind == "client_upload" {
            Some(&cur)
        } else {
            None
        };
        let stats = with_previous_trace(cur.stats.clone(), prev);
        // 云自建同样给出 head：read_stats 在读得到 .git 时写入 `stats.head`（EN-26 版本快照戳）——
        // 两入口因此落在同一坐标系（R1/R6：head 可比）。
        let head = stats
            .as_ref()
            .and_then(|s| s.get("head"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        sqlx::query(
            "UPDATE cg_projects SET source_kind = 'cloud_index', produced_at = now(), \
         built_with_version = $2, last_producer = 'cloud_index', stats = $3, \
         head = COALESCE($4, head), updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(ver)
        .bind(stats)
        .bind(head)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 失效条目对账（EN-48）：扫描全部项目，把「自称 ready 但产物已丢失 / 路径已不存在」的
    /// 条目落到 error（附具体病因），使 list 不再把幽灵条目冒充可用资产。
    ///
    /// 只改状态、不动登记与源码——重新 index 即可恢复 ready，可逆。
    ///
    /// 自愈（EN-48 残留）：返回体 `needs_rebuild` 列出「路径仍在、仅产物丢失」的条目——
    /// 调用方（HTTP/MCP 层）对它们自动入队重建 job；「路径不存在」的幽灵无法自愈，
    /// 只标 error 等人重新注册或删除。
    pub async fn gc(&self) -> Result<serde_json::Value, CgError> {
        let rows = self.list().await?;
        let mut marked = Vec::new();
        let mut needs_rebuild = Vec::new();
        for proj in rows.iter().filter(|p| p.status == "ready" && !p.usable) {
            let (reason, healable) = if !Path::new(&proj.path).exists() {
                (
                    format!(
                        "项目路径不存在（{}）——目录已删除或移动（容器形态注册的路径在宿主上不可见）；\
                     请重新注册或删除该条目",
                        proj.path
                    ),
                    false,
                )
            } else if proj.source_kind == "client_upload" {
                (
                    format!(
                        "上传产物已丢失（{}）——upload 型条目服务端无源码，无法本机重建；\
                     请客户端本机 codegraph index 后重新 upload",
                        index_db_path(Path::new(&proj.path)).display()
                    ),
                    false,
                )
            } else {
                (
                    format!(
                        "索引产物已丢失（{}）——请重新执行 codegraph index",
                        index_db_path(Path::new(&proj.path)).display()
                    ),
                    true,
                )
            };
            sqlx::query(
                "UPDATE cg_projects SET status = 'error', error = $2, updated_at = now() \
             WHERE id = $1",
            )
            .bind(proj.id)
            .bind(&reason)
            .execute(&self.pool)
            .await?;
            marked.push(serde_json::json!({
                "name": proj.name, "path": proj.path, "reason": reason,
            }));
            if healable {
                needs_rebuild.push(serde_json::json!({
                    "id": proj.id, "name": proj.name, "path": proj.path,
                }));
            }
        }
        Ok(serde_json::json!({
            "scanned": rows.len(),
            "marked_invalid": marked.len(),
            "items": marked,
            "needs_rebuild": needs_rebuild,
            "note": "置为 error 的条目：needs_rebuild 里的（路径仍在、仅产物丢失）已由服务端自动入队重建；\
                     路径不存在的幽灵条目需人工重新注册或删除",
        }))
    }

    /// 建索引（registered → indexing → ready/error）。超时 10min。
    pub async fn index(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        let proj = self.get(id).await?;
        self.ensure_version().await?;

        self.set_status(id, "indexing", None, None).await?;
        // 命令选择按「索引产物 db 是否存在」（EN-48 残留修复）：db 在 → index（增量）；
        // db 不在 → init（重建）。此前用 `.codegraph/` 目录存在性判断，但「目录在、db 丢」
        // （手动清理/备份不完整）时 CLI 的 index 会报 CodeGraph not initialized 而失败——
        // 自愈路径（gc 自动重建）恰恰专治产物丢失，必须用产物本体做判据。
        let cmd: &str = if index_db_path(Path::new(&proj.path)).exists() {
            "index"
        } else {
            "init"
        };
        match run_cli(&[cmd], Some(Path::new(&proj.path)), TIMEOUT_INIT).await {
            Ok(_) => {
                let mut stats = self.read_stats(Path::new(&proj.path)).await;
                // 初布局（t3）：算好落 `<仓库>/.codegraph/layout.json`；失败只写 stats.layout_warning
                self.merge_layout_into_stats(Path::new(&proj.path), &mut stats)
                    .await;
                self.set_status(id, "ready", stats.as_ref(), None).await?;
                // 产物元数据三件套（迁移 0055）：产出时刻 + CLI 版本 + 投递者 = cloud_index
                self.mark_cloud_artifact(id).await?;
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_status(id, "error", None, Some(&msg)).await?;
                return Err(e);
            }
        }
        self.get(id).await
    }

    /// 增量同步。超时 60s。
    pub async fn sync(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        let proj = self.get(id).await?;
        self.ensure_version().await?;
        match run_cli(&["sync"], Some(Path::new(&proj.path)), TIMEOUT_SYNC).await {
            Ok(_) => {
                let mut stats = self.read_stats(Path::new(&proj.path)).await;
                // 初布局（t3）：增量同步后结构可能变了，重算覆盖（失败只写 warning）
                self.merge_layout_into_stats(Path::new(&proj.path), &mut stats)
                    .await;
                self.set_status(id, "ready", stats.as_ref(), None).await?;
                // 产物元数据三件套（迁移 0055）：增量同步同样刷新产出时刻/版本/投递者
                self.mark_cloud_artifact(id).await?;
                sqlx::query("UPDATE cg_projects SET last_synced_at = now() WHERE id = $1")
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_status(id, "error", None, Some(&msg)).await?;
                return Err(e);
            }
        }
        self.get(id).await
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use sqlx::postgres::PgPool;

    /// 超限拒（入口收敛 2026-09-21）：产物本体 256MB 上限，两条入口（MCP/HTTP）共用这一道校验。
    /// 用惰性连接池——尺寸校验发生在任何 SQL 之前，超限时根本不碰库（本用例也不该碰）。
    #[tokio::test]
    async fn oversized_artifact_rejected_before_touching_db() {
        let pool = PgPool::connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap();
        let bridge = CgBridge::new(pool, std::path::PathBuf::from("/tmp/cg-limit-probe"));
        let mut big = vec![0u8; 256 * 1024 * 1024 + 1];
        big[..16].copy_from_slice(b"SQLite format 3\0");
        let err = bridge
            .upload_artifact("oversize", "a1b2c3d4e5", "client:test", &big)
            .await
            .expect_err("超限必须被拒（且不落盘、不碰库）");
        match err {
            CgError::BadRequest(m) => {
                assert!(m.contains("超限"), "应点明超限：{m}");
                assert!(m.contains("register"), "应指引改用 git 地址入口：{m}");
            }
            other => panic!("应 BadRequest 超限，实得 {other:?}"),
        }
    }
}
