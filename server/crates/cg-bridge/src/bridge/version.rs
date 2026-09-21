//! `bridge` 的实现切片（架构治理 2026-09-21：自 bridge.rs 纯搬移，零行为变化）。

use super::*;

impl CgBridge {
    pub fn new(pool: sqlx::PgPool, root: impl Into<PathBuf>) -> Self {
        Self {
            pool,
            root: root.into(),
        }
    }

    /// 探测 CLI 版本（失败 = CLI 不可用）。
    pub async fn detect_version(&self) -> Result<String, CgError> {
        let out = run_cli(&["version"], None, TIMEOUT_QUERY).await?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if v.is_empty() {
            return Err(CgError::Parse("version 输出为空".into()));
        }
        Ok(v)
    }

    /// 版本守卫：pin 不符 → VersionMismatch。
    pub async fn ensure_version(&self) -> Result<(), CgError> {
        let v = self.detect_version().await?;
        // 兼容 "1.5.0" 与 "codegraph 1.5.0" 形态
        let ver = v.split_whitespace().last().unwrap_or(&v);
        if ver != CG_VERSION_PIN {
            return Err(CgError::VersionMismatch {
                need: CG_VERSION_PIN.into(),
                got: ver.into(),
            });
        }
        Ok(())
    }

    /// 读索引入口的**唯一门禁**（EN-48）：query / full_graph / graph 都过这里，
    /// 按病因给不同且可行动的错误——不再是「同一故障三个入口三种表现」。
    ///
    /// 判定顺序即病因优先级：版本 → 状态 → 路径 → 产物。返回索引库路径（已确认在盘）。
    pub(crate) fn ensure_ready(&self, proj: &CgProjectDto) -> Result<PathBuf, CgError> {
        if proj.status == "version_mismatch" {
            return Err(CgError::VersionMismatch {
                need: CG_VERSION_PIN.into(),
                got: "unknown".into(),
            });
        }
        if proj.status != "ready" {
            let hint = match proj.status.as_str() {
                "registered" => "从未建过索引——先执行 codegraph index",
                "indexing" => "索引仍在进行中——稍后 codegraph list 确认 ready",
                "error" => "上次索引失败——看 codegraph list 的 error 字段，修好后重新 index",
                _ => "先执行 codegraph index",
            };
            return Err(CgError::BadRequest(format!(
                "项目未就绪（{}）——{hint}",
                proj.status
            )));
        }
        // ① 路径：目录被删/搬走，或条目来自容器形态（宿主看不到该路径）
        let path = Path::new(&proj.path);
        if !path.exists() {
            return Err(CgError::NotFound(format!(
                "项目路径不存在（{}）——目录可能已被删除或移动（容器形态下注册的路径在宿主上不可见）；\
             请重新注册，或删除该条目",
                proj.path
            )));
        }
        // ② 产物：status=ready 只是「历史上成功过」，`.codegraph/` 一旦随目录消失不会自己回来
        let db_path = index_db_path(path);
        if !db_path.is_file() {
            return Err(CgError::NotFound(format!(
                "索引产物已丢失（{}）——`.codegraph/` 是 codegraph CLI 的未跟踪产物，\
             目录被重新 clone / 清理后不会自己回来（注册状态仍是 ready，因为那只是历史记录）；\
             请重新执行 codegraph index",
                db_path.display()
            )));
        }
        Ok(db_path)
    }

    pub(crate) async fn set_status(
        &self,
        id: Uuid,
        status: &str,
        stats: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> Result<(), CgError> {
        sqlx::query("UPDATE cg_projects SET status = $2, stats = $3, error = $4, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .bind(stats.map(sqlx::types::Json))
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 索引统计：CLI status --json 归一为前端契约 {files, symbols, edges, by_kind}。
    /// （CLI 字段是 fileCount/nodeCount/edgeCount——此前直透导致前端恒显示 ?。）
    /// 另附 `head`：index/sync 完成时刻的 HEAD commit hash（版本快照戳，EN-26）——
    /// 读得到 .git 才写；此后即使 repo 摘走，快照语义（「图是哪个 commit 的」）仍在 stats 里。
    pub(crate) async fn read_stats(&self, path: &Path) -> Option<serde_json::Value> {
        let out = run_cli(&["status", "--json"], Some(path), TIMEOUT_QUERY)
            .await
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        // 快照戳：构建时刻的 HEAD（读不到 = 非 git 仓库，留 null 不阻塞统计）
        let head = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        Some(serde_json::json!({
            "files": v.get("fileCount")?,
            "symbols": v.get("nodeCount")?,
            "edges": v.get("edgeCount")?,
            "by_kind": v.get("nodesByKind").cloned().unwrap_or(serde_json::json!({})),
            "last_indexed": v.get("lastIndexed").cloned().unwrap_or(serde_json::Value::Null),
            "head": head,
        }))
    }

    /// CLI 可用性（前端状态条）：版本探测失败 = 不可用。
    /// `hint`（R5，task-5）：不可用 / 版本不符都给可照抄的「装 + 锁版」指引；正常为 None。
    pub async fn cli_status(&self) -> CliStatus {
        match self.detect_version().await {
            Ok(v) => {
                // 兼容 "1.5.0" 与 "codegraph 1.5.0" 形态（与 ensure_version 同口径）；
                // 取 owned String——否则 `ver` 借用 `v` 与后面把 `v` move 进 version 冲突（E0505）
                let ver = v.split_whitespace().last().unwrap_or(&v).to_string();
                let mismatch = ver != CG_VERSION_PIN;
                CliStatus {
                    available: true,
                    version: Some(v),
                    pin: CG_VERSION_PIN.into(),
                    hint: mismatch.then(|| {
                        format!(
                            "本机 CLI 版本 {ver} ≠ pin {CG_VERSION_PIN}——{}",
                            cli_fix_hint(CG_VERSION_PIN)
                        )
                    }),
                }
            }
            Err(e) => CliStatus {
                available: false,
                version: None,
                pin: CG_VERSION_PIN.into(),
                hint: Some(format!("{e}——{}", cli_fix_hint(CG_VERSION_PIN))),
            },
        }
    }

    /// 标记全部项目版本不匹配（CLI 升级后调用）。
    pub async fn mark_all_version_mismatch(&self, actual: &str) -> Result<u64, CgError> {
        let r = sqlx::query(
        "UPDATE cg_projects SET status = 'version_mismatch', error = $1, updated_at = now() WHERE status != 'version_mismatch'",
    )
    .bind(format!("CLI 版本 {actual} != pin {CG_VERSION_PIN}"))
    .execute(&self.pool)
    .await?;
        Ok(r.rows_affected())
    }
}
