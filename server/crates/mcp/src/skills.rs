//! skills 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

// ---------- 技能域工具参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsListParams {
    /// 可选关键词（搜名称与描述）
    #[schemars(description = "可选：关键词，模糊匹配技能名称与描述。")]
    pub q: Option<String>,
    /// 可选标签过滤
    #[schemars(description = "可选：按标签过滤（含该标签即命中）。")]
    pub tag: Option<String>,
    /// 可选启用过滤：true=只看启用 / false=只看停用 / 缺省=全部
    #[schemars(description = "可选：true 只看启用，false 只看停用，缺省全部。")]
    pub enabled: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsGetParams {
    /// 技能 slug（kebab-case 标识）
    #[schemars(
        description = "技能 slug（来自 skills_list 的返回，如 review-pr）。也接受技能名 name 精确匹配。"
    )]
    pub slug: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsFileGetParams {
    /// 技能 slug（来自 skills_list 的返回）
    #[schemars(description = "技能 slug（来自 skills_list 的返回）。")]
    pub slug: String,
    /// 附属文件相对路径（来自 skills_get 返回的 files 索引）
    #[schemars(description = "附属文件相对路径（/ 分隔，如 scripts/run.py、references/api.md）。")]
    pub path: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsFilePutParams {
    /// 技能 slug（来自 skills_list 的返回）
    #[schemars(description = "技能 slug（来自 skills_list 的返回）。")]
    pub slug: String,
    /// 附属文件相对路径（/ 分隔；禁止 .. 与绝对路径；SKILL.md 本体走 skills_update）
    #[schemars(
        description = "附属文件相对路径（/ 分隔，如 scripts/run.py）。禁止 .. 与绝对路径；SKILL.md 本体走 skills_update。"
    )]
    pub path: String,
    /// 文件文本内容（脚本/参考资料/模板）
    #[schemars(description = "文件文本内容（脚本/参考资料/模板）。同路径重复写 = 覆盖更新。")]
    pub content: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsCreateParams {
    /// 技能名
    #[schemars(description = "技能名（简短、可辨认，如「PR 审查」）。")]
    pub name: String,
    /// markdown 正文（技能指令本体）
    #[schemars(
        description = "技能正文，markdown。写清这个技能做什么、怎么做、何时用。kind=script 时传空字符串（正文不入库——SKILL.md 真身在 local_path，系统只存指针）。"
    )]
    pub content: String,
    /// 可选 slug（缺省从 name 推导；中文/非 ASCII 名必须显式给）
    #[schemars(
        description = "可选：slug（kebab-case 标识，如 review-pr）。缺省从 name 推导；name 非 ASCII 时必须显式给。"
    )]
    pub slug: Option<String>,
    /// 可选一句话描述
    #[schemars(description = "可选：一句话描述这个技能做什么、何时该用（列表与选择时的依据）。")]
    pub description: Option<String>,
    /// 可选标签
    #[schemars(description = "可选：标签列表，便于分类过滤。")]
    pub tags: Option<Vec<String>>,
    /// 可选：存储形态（text/script）
    #[schemars(
        description = "可选：text=整体入库（默认，纯文本技能，依赖走 npm/cargo 全局二进制时也用这个）；script=带 .py/.sh 等真脚本的技能——真身存本地文件夹（SKILL.md+scripts/），库中只存指针，正文不入库（get 现读），file/versions 操作不可用。"
    )]
    pub kind: Option<String>,
    /// 可选：来源（self/github/both）
    #[schemars(
        description = "可选：self=自建未发布（默认）/ github=源自 GitHub / both=自建且已发布。"
    )]
    pub origin: Option<String>,
    /// script 型必填：本地技能文件夹路径
    #[schemars(
        description = "kind=script 时必填：本地技能文件夹绝对路径（含 SKILL.md），系统只存指针。kind=text 时不要传。"
    )]
    pub local_path: Option<String>,
    /// 可选：GitHub 仓库地址（元数据）
    #[schemars(
        description = "可选：GitHub 仓库地址（origin=github 或 both 时填）。纯元数据，不做远端拉取。"
    )]
    pub repo_url: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsUpdateParams {
    /// 目标技能 slug
    #[schemars(description = "要更新的技能 slug。")]
    pub slug: String,
    /// 可选：改技能名
    #[schemars(description = "可选：新技能名。不传不动。")]
    pub name: Option<String>,
    /// 可选：改描述
    #[schemars(description = "可选：新描述。不传不动。")]
    pub description: Option<String>,
    /// 可选：改正文
    #[schemars(description = "可选：新正文（markdown，整体替换）。不传不动。")]
    pub content: Option<String>,
    /// 可选：改标签
    #[schemars(description = "可选：新标签列表（整体替换）。不传不动。")]
    pub tags: Option<Vec<String>>,
    /// 可选：启用/停用
    #[schemars(
        description = "可选：true=启用 / false=停用。停用后不再出现在 enabled=true 过滤里；缺省列表是管理视角仍会显示（停用技能不被删除）。"
    )]
    pub enabled: Option<bool>,
    /// 可选：改来源（self/github/both）
    #[schemars(
        description = "可选：改来源。self=自建未发布 / github=源自 GitHub / both=自建且已发布。origin 改回 self 时 repo_url 自动清空。"
    )]
    pub origin: Option<String>,
    /// 可选：改仓库地址
    #[schemars(description = "可选：改 GitHub 仓库地址（origin 含 github 时有意义）。")]
    pub repo_url: Option<String>,
    /// 可选：script 型指针改址
    #[schemars(description = "可选：script 型技能改本地路径（指针搬家）。text 型不可用。")]
    pub local_path: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsImportParams {
    /// SKILL.md 全文
    #[schemars(
        description = "SKILL.md 全文：可选 --- frontmatter（name/description/slug/tags 键）+ markdown 正文。没有 frontmatter 时需传 filename 或 name 兜底。"
    )]
    pub content: String,
    /// 可选文件名（无 frontmatter name 时兜底命名）
    #[schemars(
        description = "可选：来源文件名（如 review-pr.md），无 frontmatter name 时用来兜底命名。"
    )]
    pub filename: Option<String>,
    /// 可选：技能名（frontmatter 与 filename 都没有时兜底）
    #[schemars(description = "可选：技能名兜底（frontmatter name 与 filename 都缺时必须给）。")]
    pub name: Option<String>,
    /// 命中已有 slug 时覆盖更新（默认 false）
    #[schemars(description = "可选：slug 已存在时是否覆盖更新，默认 false（该条报错）。")]
    pub overwrite: Option<bool>,
}

/// skills_list 动态段：枚举启用技能（slug：name/description）。
/// 云部署后的技能发现入口——库里有什么，AI 连上来第一眼就看到。
pub(crate) async fn skills_catalog(pool: &engram_storage::PgPool) -> Option<String> {
    let rows = engram_storage::repo::skills::list_skills(pool, None, None, Some(true))
        .await
        .ok()?;
    if rows.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = rows
        .iter()
        .take(CATALOG_ITEM_CAP)
        .map(|s| {
            let desc = if s.description.trim().is_empty() {
                s.name.as_str()
            } else {
                s.description.as_str()
            };
            format!("- {}：{}", s.slug, desc)
        })
        .collect();
    if rows.len() > CATALOG_ITEM_CAP {
        lines.push(format!(
            "…其余 {} 个请调用本工具查看完整清单",
            rows.len() - CATALOG_ITEM_CAP
        ));
    }
    Some(format!(
        "【当前可用技能 {} 个】（命中候选后用 skills 的 action=\"get\" 取全文照做）\n{}",
        rows.len(),
        lines.join("\n")
    ))
}

#[tool_router(router = skills_router)]
impl EngramMcpServer {
    // ---------- 技能域工具（可复用指令包：SKILL.md 形态） ----------

    /// 列出技能（AI 技能库的浏览入口；q/tag/enabled 过滤，不含正文）。
    ///
    /// 何时用：开始需要某种可复用能力前，先看库里有没有现成技能；或按标签浏览技能面。
    /// 命中候选后用 skills_get 取正文照做；没有合适的再用 skills_create 沉淀新技能。
    pub(crate) async fn skills_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let lp = params.0;
        let rows = self
            .skills_svc()
            .list_skills(lp.q.as_deref(), lp.tag.as_deref(), lp.enabled)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个技能全文（正文即指令——照做即可复用该技能）。
    ///
    /// 何时用：skills_list 命中候选后，取全文执行；或用户点名某个技能时。
    /// 消费形态指南（按需选通道，不要一股脑拉全量）：
    /// ① 纯文本技能 → 本工具读正文即完事；② 需要某个附属文件（脚本/参考资料）→
    /// skills_file_get 看内容，要落盘就 HTTP 直下：curl -s -H "Authorization: Bearer $KEY"
    /// "{本服务origin}/skills/{slug}/file?path=…&raw=1" -o 文件名；
    /// ③ 需要整个文件夹（SKILL.md+scripts+references）→
    /// curl -s -H "Authorization: Bearer $KEY" "{origin}/skills/{slug}/bundle" -o s.zip && tar -xf s.zip。
    pub(crate) async fn skills_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        // EN-54：script 型从 local_path 现读 SKILL.md 正文（与 HTTP 层 GET /skills/{slug} 对齐）。
        // 此前调 get_skill（不带正文）且无条件 list_files 被 script 守卫拒掉，
        // 报出张冠李戴的「文件列表 对 script 型技能不可用」——与 help 承诺的「现读」直接矛盾。
        let s = self
            .skills_svc()
            .get_skill_with_content(&params.0.slug)
            .await
            .map_err(from_skills)?;
        // folder 形态：附属文件索引随详情下发（AI 据此用 skills_file_get 取脚本/参考资料）；
        // script 型附件在本地（指针语义），无服务端文件列表——跳过而非报错
        let files = if s.kind == "script" {
            Vec::new()
        } else {
            self.skills_svc()
                .list_files(&params.0.slug)
                .await
                .map_err(from_skills)?
        };
        let mut v = serde_json::to_value(&s).unwrap_or(serde_json::json!({}));
        v["files"] = serde_json::to_value(&files).unwrap_or(serde_json::json!([]));
        ok_json(v)
    }

    /// 读取技能附属文件（scripts/ / references/ 等按路径寻址的文件）。
    ///
    /// 何时用：SKILL.md（skills_get 的 content）里引用了 scripts/xxx.py、
    /// references/api.md 等相对路径时——取到的是文件内容，脚本由客户端本地执行
    /// （服务端只存内容，永不代执行）。
    /// 只需要一个文件且要落盘时，HTTP 直下比逐文件调本工具更快：
    /// curl -s -H "Authorization: Bearer $KEY" "{本服务origin}/skills/{slug}/file?path=…&raw=1" -o 文件名
    /// （需要整个文件夹时用整包：curl … "{origin}/skills/{slug}/bundle" -o s.zip && tar -xf s.zip）
    pub(crate) async fn skills_file_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsFileGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let content = self
            .skills_svc()
            .get_file(&params.0.slug, &params.0.path)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::json!({
            "slug": params.0.slug,
            "path": params.0.path,
            "content": content,
        }))
    }

    /// 写（upsert）技能附属文件：沉淀技能时把脚本/参考资料一并入库。
    ///
    /// 何时用：skills_create 之后补充 scripts/、references/ 等文件；
    /// 同路径重复写 = 覆盖更新。SKILL.md 本体走 skills_update 的 content。
    pub(crate) async fn skills_file_put(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsFilePutParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let (path, size) = self
            .skills_svc()
            .put_file(&params.0.slug, &params.0.path, &params.0.content)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::json!({
            "slug": params.0.slug,
            "path": path,
            "size": size,
        }))
    }

    /// 沉淀新技能：把本次对话中验证有效的做法固化成可复用指令包。
    ///
    /// 何时用：用户说「把这个做法存成技能/记成 SOP」，或一套流程已被验证有效且可复用时。
    /// 何时不用：一次性的操作细节不值得建技能；用户个人事实走记忆域（memory_write_session）。
    pub(crate) async fn skills_create(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let cp = params.0;
        let s = self
            .skills_svc()
            .create_skill(engram_core::skills::NewSkill {
                slug: cp.slug.as_deref(),
                name: &cp.name,
                description: cp.description.as_deref().unwrap_or(""),
                content: &cp.content,
                tags: &cp.tags.unwrap_or_default(),
                enabled: true,
                source: "mcp",
                kind: cp.kind.as_deref().unwrap_or("text"),
                origin: cp.origin.as_deref().unwrap_or("self"),
                local_path: cp.local_path.as_deref(),
                repo_url: cp.repo_url.as_deref(),
            })
            .await
            .map_err(from_skills)?;
        ok_json(slim_skill(
            serde_json::to_value(&s).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 更新技能（正文/名称/描述/标签/启停；语义变更自动留版本快照，可回滚）。
    ///
    /// 何时用：技能做法需要修正或演进时（如用户指出了更好的步骤）。
    /// 注意：改坏可回滚（版本快照），但删除不可逆——拿不准就更新而不是删除。
    pub(crate) async fn skills_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let up = params.0;
        let s = self
            .skills_svc()
            .update_skill(
                &up.slug,
                engram_core::skills::SkillPatch {
                    name: up.name,
                    description: up.description,
                    content: up.content,
                    tags: up.tags,
                    enabled: up.enabled,
                    origin: up.origin,
                    repo_url: up.repo_url,
                    local_path: up.local_path,
                },
            )
            .await
            .map_err(from_skills)?;
        ok_json(slim_skill(
            serde_json::to_value(&s).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 导入一个 SKILL.md（frontmatter 容错解析——迁移现有技能库零改写）。
    ///
    /// 何时用：用户给了现成的 SKILL.md 文件/文本要入库时。逐条导入用本工具，
    /// 批量走 HTTP API POST /skills/import（逐条报告，互不阻断）。
    pub(crate) async fn skills_import(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsImportParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let ip = params.0;
        let (meta, body) = engram_core::skills::parse_frontmatter(&ip.content);
        let name = meta
            .name
            .or(ip.name)
            .or(ip.filename.clone())
            .unwrap_or_default();
        let name = name
            .trim_end_matches(".md")
            .trim_end_matches(".markdown")
            .to_string();
        if name.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "无法确定技能名——请在 frontmatter 写 name，或传 filename/name 兜底",
            ));
        }
        // slug：frontmatter > 名字推导（中文名推导失败 → 显式报错，不让坏 slug 落库）
        let slug = match meta.slug.or_else(|| engram_core::skills::slugify(&name)) {
            Some(s) => s,
            None => {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("无法从技能名「{name}」推导 slug——请显式给 slug（kebab-case）"),
                ));
            }
        };
        let overwrite = ip.overwrite.unwrap_or(false);
        let result = if self.skills_svc().get_skill(&slug).await.is_ok() && overwrite {
            self.skills_svc()
                .update_skill(
                    &slug,
                    engram_core::skills::SkillPatch {
                        name: Some(name),
                        description: Some(meta.description.unwrap_or_default()),
                        content: Some(body),
                        tags: Some(meta.tags),
                        enabled: None,
                        origin: None,
                        repo_url: None,
                        local_path: None,
                    },
                )
                .await
                .map(|s| ("updated", s))
        } else {
            self.skills_svc()
                .create_skill(engram_core::skills::NewSkill {
                    slug: Some(&slug),
                    name: &name,
                    description: &meta.description.unwrap_or_default(),
                    content: &body,
                    tags: &meta.tags,
                    enabled: true,
                    source: "mcp",
                    kind: "text",
                    origin: "self",
                    local_path: None,
                    repo_url: None,
                })
                .await
                .map(|s| ("imported", s))
        };
        let (status, s) = result.map_err(from_skills)?;
        ok_json(serde_json::json!({
            "status": status,
            "skill": slim_skill(serde_json::to_value(&s).unwrap_or(serde_json::json!({}))),
        }))
    }

    /// 技能域（单一入口）：技能 = 可复用的指令包（SKILL.md 形态 + scripts/references 附件）。
    /// 需要某种能力前先 "list" 找现成的，命中 "get" 取全文照做；验证有效的做法
    /// 用 "create" 沉淀。操作全景：action="help"。
    #[tool(
        name = "skills",
        annotations(
            title = "技能域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn skills_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("skills", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "list" => {
                self.skills_list(
                    ctx,
                    Parameters(dispatch::from_args("skills", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.skills_get(
                    ctx,
                    Parameters(dispatch::from_args("skills", "get", call.args)?),
                )
                .await
            }
            "file_get" => {
                self.skills_file_get(
                    ctx,
                    Parameters(dispatch::from_args("skills", "file_get", call.args)?),
                )
                .await
            }
            "file_put" => {
                self.skills_file_put(
                    ctx,
                    Parameters(dispatch::from_args("skills", "file_put", call.args)?),
                )
                .await
            }
            "create" => {
                self.skills_create(
                    ctx,
                    Parameters(dispatch::from_args("skills", "create", call.args)?),
                )
                .await
            }
            "update" => {
                self.skills_update(
                    ctx,
                    Parameters(dispatch::from_args("skills", "update", call.args)?),
                )
                .await
            }
            "versions" => {
                self.skills_versions(
                    ctx,
                    Parameters(dispatch::from_args("skills", "versions", call.args)?),
                )
                .await
            }
            "restore" => {
                self.skills_restore(
                    ctx,
                    Parameters(dispatch::from_args("skills", "restore", call.args)?),
                )
                .await
            }
            "delete" => {
                self.skills_delete(
                    ctx,
                    Parameters(dispatch::from_args("skills", "delete", call.args)?),
                )
                .await
            }
            "import" => {
                self.skills_import(
                    ctx,
                    Parameters(dispatch::from_args("skills", "import", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("skills", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_skills() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::skills_router()
}
