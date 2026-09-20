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
    /// 注册项目：本地路径（须存在）或 git URL（clone depth 1 到工作目录）。
    pub async fn register(&self, name: &str, source_uri: &str) -> Result<CgProjectDto, CgError> {
        let existing: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM cg_projects WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        if existing.is_some() {
            return Err(CgError::BadRequest(format!("项目名 {name} 已存在")));
        }
        // 同一来源（路径/仓库）只许注册一次——避免同库多份索引
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

        let id = Uuid::now_v7();
        let workdir = self.root.join(id.to_string());
        let (path, uri) = if source_uri.starts_with("http://")
            || source_uri.starts_with("https://")
            || source_uri.ends_with(".git")
        {
            // git clone --depth 1
            tokio::fs::create_dir_all(&self.root).await.ok();
            let out = tokio::process::Command::new("git")
                .args(["clone", "--depth", "1", source_uri])
                .arg(&workdir)
                .output()
                .await
                .map_err(|e| CgError::BadRequest(format!("git 不可用: {e}")))?;
            if !out.status.success() {
                return Err(CgError::BadRequest(format!(
                    "clone 失败: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(300)
                        .collect::<String>()
                )));
            }
            (
                workdir.to_string_lossy().into_owned(),
                source_uri.to_string(),
            )
        } else {
            let p = Path::new(source_uri);
            if !p.exists() {
                return Err(CgError::BadRequest(format!(
                    "本地路径不存在: {source_uri}——注意路径按**服务端**文件系统校验 \
                 （MCP 客户端在另一台机器上时，它本地的路径服务端看不到，请改用 git URL）"
                )));
            }
            (source_uri.to_string(), source_uri.to_string())
        };

        let row = sqlx::query_as::<_, CgProjectDto>(
            "INSERT INTO cg_projects (id, name, path, source_uri, status) \
         VALUES ($1, $2, $3, $4, 'registered') RETURNING *",
        )
        .bind(id)
        .bind(name)
        .bind(&path)
        .bind(&uri)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// 产物上传（公网多Agent P001 步骤4）：客户端本机 codegraph CLI index 后，
    /// 上传 `.codegraph/codegraph.db` + HEAD——服务端只存产物 + 声明式新鲜度
    /// （head/uploaded_at），无代码、无 git 凭证。CLI 是基础设施（CG_VERSION_PIN），
    /// 客户端宿主零依赖。
    ///
    /// name 不存在则新建条目（status=ready）；存在且为 upload 型则覆盖产物；
    /// repo 型拒绝覆盖（本机索引不归上传通道管）。
    pub async fn upload_artifact(
        &self,
        name: &str,
        head: &str,
        db_bytes: &[u8],
    ) -> Result<CgProjectDto, CgError> {
        // 校验：head 是 commit hash（7~40 位 hex，短/长 SHA 都收）
        let head = head.trim();
        if !(7..=40).contains(&head.len()) || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CgError::BadRequest(format!(
                "head 不是合法 commit hash（7~40 位 hex，收到 {} 字符）——客户端本机 `git rev-parse HEAD` 取",
                head.len()
            )));
        }
        // 校验：SQLite 魔数（坏产物当场拒——「能存进去但查不了」才是最差的体验）
        const SQLITE_MAGIC: &[u8] = b"SQLite format 3\x00";
        if db_bytes.len() < SQLITE_MAGIC.len() || &db_bytes[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
            return Err(CgError::BadRequest(
                "db 不是 SQLite 文件（缺「SQLite format 3」魔数）——请上传 codegraph CLI 产出的 \
             .codegraph/codegraph.db 本体（原始二进制，不要压缩/文本化）"
                    .into(),
            ));
        }
        // 上限 256MB（单用户系统，一次 HTTP body 可承载；再大说明仓库该拆了）
        if db_bytes.len() > 256 * 1024 * 1024 {
            return Err(CgError::BadRequest(format!(
                "db 超限（{} MB > 256 MB）——拆分仓库或精简索引范围后重传",
                db_bytes.len() / 1024 / 1024
            )));
        }

        let existing: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id, source_kind FROM cg_projects WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        let id = match existing {
            Some((id, kind)) => {
                if kind != "upload" {
                    return Err(CgError::BadRequest(format!(
                        "项目 {name} 是 repo 型（服务端本机索引）——产物上传只作用于 upload 型条目；\
                     请换名注册，或先 delete 再以 upload 重建"
                    )));
                }
                id
            }
            None => {
                let nid = Uuid::now_v7();
                let dir = self
                    .root
                    .join("uploads")
                    .join(nid.to_string())
                    .join(".codegraph");
                sqlx::query_as::<_, CgProjectDto>(
                    "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind) \
                 VALUES ($1, $2, $3, $4, 'ready', 'upload') RETURNING *",
                )
                .bind(nid)
                .bind(name)
                .bind(
                    dir.parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
                .bind(format!("upload://{head}"))
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
        tokio::fs::rename(&tmp, &dst)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;

        // 声明式新鲜度：head + uploaded_at，status 直接 ready（产物确在盘）
        sqlx::query(
            "UPDATE cg_projects SET head = $2, uploaded_at = now(), status = 'ready', \
         error = NULL, last_synced_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(head)
        .execute(&self.pool)
        .await?;
        self.get(id).await
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
            } else if proj.source_kind == "upload" {
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
                let stats = self.read_stats(Path::new(&proj.path)).await;
                self.set_status(id, "ready", stats.as_ref(), None).await?;
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
                let stats = self.read_stats(Path::new(&proj.path)).await;
                self.set_status(id, "ready", stats.as_ref(), None).await?;
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
