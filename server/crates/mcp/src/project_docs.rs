//! project_docs 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocAddParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 分类名（须是项目已有分类；新分类先 project_update 追加）
    #[schemars(
        description = "分类名，必须是项目已有分类（先 project_get 看 categories）。要新分类就先 project_update 把它加进 categories。同项目同分类下 title 唯一。"
    )]
    pub category: String,
    /// 文档标题
    #[schemars(description = "文档标题（同项目同分类同 folder 下唯一）。")]
    pub title: String,
    /// 可选：子文件夹相对路径
    #[schemars(
        description = "可选：子文件夹相对路径（/ 分隔多级，如 审计、归档/ai-permissions；'' = 分类根下）。树形呈现：分类 → folder → 文档。"
    )]
    pub folder: Option<String>,
    /// Markdown 正文
    #[schemars(description = "Markdown 正文。沉淀进展、结论、决策时写清楚背景与结果。")]
    pub content: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocGetParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
    /// 起始行（1-based；与 end_line 搭配精确读一个区间）
    #[schemars(
        description = "可选：起始行号（1-based，含该行）。与 end_line 搭配做区间精读；行号来自 project_doc_search 的命中或 with_line_numbers 的全文。不传 = 从头。"
    )]
    pub start_line: Option<i64>,
    /// 结束行（1-based，含该行）
    #[schemars(description = "可选：结束行号（1-based，含该行）。不传 = 到末尾。")]
    pub end_line: Option<i64>,
    /// 输出加行号前缀（区间模式恒带行号；全文默认不加）
    #[schemars(
        description = "可选：true = 全文每行加「行号: 」前缀，便于后续按行寻址。不传或 false = 原文。区间读取（传了 start_line/end_line）恒带行号。"
    )]
    pub with_line_numbers: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocSearchParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 检索词（按行大小写不敏感子串匹配）
    #[schemars(description = "检索词：按行大小写不敏感子串匹配（grep 式）。")]
    pub query: String,
    /// 命中上限（默认 50）
    #[schemars(description = "命中上限，默认 50。命中含 doc_id/title/category/line/text。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocUpdateParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
    /// 新分类
    #[schemars(description = "可选：移到新分类（须是项目已有分类）。不传不改。")]
    pub category: Option<String>,
    /// 新子文件夹
    #[schemars(
        description = "可选：改子文件夹相对路径（/ 分隔多级；'' = 移到分类根下）。不传不改。"
    )]
    pub folder: Option<String>,
    /// 新标题
    #[schemars(description = "可选：新标题。不传不改。")]
    pub title: Option<String>,
    /// 新正文（替换式；先 project_doc_get 取原文再追加修改）
    #[schemars(
        description = "可选：替换整个 Markdown 正文（是替换不是追加——改长文档先 project_doc_get 取原文）。不传不改。"
    )]
    pub content: Option<String>,
    /// 乐观锁：基于的版本号（doc_get 返回的 version）。给出且与当前不符 → 报版本冲突
    #[schemars(
        description = "可选：乐观锁版本号（doc_get 返回的 version）。给出且与当前不符时报「版本冲突」——防并发覆盖。不传 = 不校验。"
    )]
    pub expected_version: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocDeleteParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
}

/// 行级补丁（R 报告 P1-10）：改长文档不再「doc_get 取全文→doc_update 重发全文」。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocPatchParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
    /// 起始行（1-based；或改用 anchor 锚点定位）
    #[schemars(
        description = "行号（1-based）。replace/delete = 区间起点；insert = 插入位置（在该行之前，total+1 = 追加到末尾）。与 anchor 二选一：给了 anchor 则忽略本参数。"
    )]
    #[serde(default)]
    pub start_line: Option<i64>,
    /// 结束行（1-based，含该行；缺省 = 与 start_line 同行）
    #[schemars(
        description = "行号（1-based，含该行）。replace/delete = 区间终点；insert 忽略此参数。缺省 = 与 start_line 同行。"
    )]
    #[serde(default)]
    pub end_line: Option<i64>,
    /// 内容锚（EN-226）：按行包含匹配定位 start_line——多 patch 串行不再漂移
    #[schemars(
        description = "可选：内容锚（doc_get 看到的原文短语）。按行包含匹配定位，唯一命中才执行；零命中/多命中报错并列出命中行。给了 anchor 则忽略 start_line；end_line 缺省 = 锚行。"
    )]
    pub anchor: Option<String>,
    /// 区间终点锚（EN-226 审计补强）：与 anchor 配对锚定多行区间——insert 后终点同样不漂移
    #[schemars(
        description = "可选：区间终点锚（原文短语）。anchor 定起点、anchor_end 定终点（唯一命中才执行）——多行 replace 在串行 insert 后区间两端都不漂移。缺省终点 = 锚行自身。"
    )]
    pub anchor_end: Option<String>,
    /// replace（默认）| insert | delete
    #[schemars(
        description = "补丁模式：\"replace\"（默认，[start_line,end_line] 替换为 content）/ \"insert\"（在 start_line 前插入 content，可传 start_line=total+1 追加）/ \"delete\"（删除 [start_line,end_line]，忽略 content）/ \"replace_text\"（EN-226：old_text 匹配式替换——anchor 携带被替换原文可跨行，content 为新文，全文唯一命中才执行；此时忽略 start_line/end_line）。"
    )]
    pub mode: Option<String>,
    /// 替换/插入的文本（可多行；delete 忽略）
    #[schemars(description = "替换或插入的文本（可多行）。mode=delete 时不需要。")]
    pub content: Option<String>,
    /// 乐观锁：基于的版本号。给出且与当前不符 → 报版本冲突
    #[schemars(
        description = "可选：乐观锁版本号（doc_get 返回的 version）。给出且与当前不符时报「版本冲突」——行号基于旧版本时防错改。不传 = 不校验。"
    )]
    pub expected_version: Option<i64>,
}

#[tool_router(router = project_docs_router)]
impl EngramMcpServer {
    /// 项目下新增分类文档（Markdown）。
    ///
    /// 何时用：沉淀进展/结论/决策——干活中的发现写成文档，收尾写总结。
    /// category 必须是项目已有分类（先 project_get 看 categories）；同项目同分类下标题唯一。
    pub(crate) async fn project_doc_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = self
            .resolve_project(&dp.project_id, &dp.project_name)
            .await?;
        // 分类归属由 service 校验（防孤儿分类，报错列出现有分类）
        let doc = self
            .svc_project()
            .add_doc(
                id,
                &dp.category,
                dp.folder.as_deref().unwrap_or(""),
                &dp.title,
                &dp.content,
            )
            .await
            .map_err(from_project)?;
        // P0-1：刚发送的正文不回显
        ok_json(slim_doc(
            serde_json::to_value(&doc).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 读取项目文档：全文或按行区间精读（1-based，含两端）。
    ///
    /// 何时用：project_get 索引或 project_doc_search 命中之后精确读内容。
    /// 传 start_line/end_line 只读该区间（输出恒带「行号: 」前缀，便于连环寻址）；
    /// 不传读全文（无损，不截断），with_line_numbers=true 可给全文加行号。
    /// 行号基于文档当前版本——改文档后需重取。
    pub(crate) async fn project_doc_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = Uuid::parse_str(&dp.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        let ranged = dp.start_line.is_some() || dp.end_line.is_some();
        let numbered = |line: i64, text: &str| format!("{line}: {text}");
        if ranged {
            let (total, lines) = self
                .svc_project()
                .read_doc_lines(id, dp.start_line, dp.end_line)
                .await
                .map_err(from_project)?;
            let start = dp.start_line.unwrap_or(1);
            let end = dp.end_line.unwrap_or(total).min(total);
            let content = lines
                .iter()
                .map(|(l, t)| numbered(*l, t))
                .collect::<Vec<_>>()
                .join("\n");
            return ok_json(json!({
                "doc_id": dp.doc_id,
                "total_lines": total,
                "start_line": start,
                "end_line": end,
                "content": content,
            }));
        }
        let doc = self.svc_project().get_doc(id).await.map_err(from_project)?;
        let total = doc.content.lines().count() as i64;
        let mut v = serde_json::to_value(&doc).unwrap_or(serde_json::json!({}));
        if let Some(obj) = v.as_object_mut() {
            obj.insert("total_lines".into(), json!(total));
            if dp.with_line_numbers.unwrap_or(false) {
                let numbered_content = doc
                    .content
                    .lines()
                    .enumerate()
                    .map(|(i, t)| numbered(i as i64 + 1, t))
                    .collect::<Vec<_>>()
                    .join("\n");
                obj.insert("content".into(), json!(numbered_content));
            }
        }
        ok_json(v)
    }

    /// grep 式跨文档按行检索项目文档（大小写不敏感子串）。
    ///
    /// 何时用：索引模式下想找「某句话/某个结论在哪篇文档哪一行」。
    /// 返回命中 {doc_id, title, category, line, text}；拿到行号后用
    /// project_doc_get 的 start_line/end_line 区间精读上下文。
    pub(crate) async fn project_doc_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let sp = params.0;
        let id = self
            .resolve_project(&sp.project_id, &sp.project_name)
            .await?;
        let hits = self
            .svc_project()
            .search_doc_lines(id, &sp.query, sp.limit.unwrap_or(50))
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&hits).unwrap_or(serde_json::json!([])))
    }

    /// 编辑项目文档（补丁式：只传要改的字段）。
    ///
    /// 何时用：追加进展、更新结论。content 是替换式——改长文档先 project_doc_get
    /// 取原文改好再整体传回。移分类时 category 须是项目已有分类。
    pub(crate) async fn project_doc_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = Uuid::parse_str(&dp.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        // 部分更新直传 Option：SQL 层 COALESCE——并发各字段互不覆盖（D2 修复）
        let doc = self
            .svc_project()
            .update_doc(
                id,
                dp.category.as_deref(),
                dp.folder.as_deref(),
                dp.title.as_deref(),
                dp.content.as_deref(),
                dp.expected_version,
            )
            .await
            .map_err(from_project)?;
        ok_json(slim_doc(
            serde_json::to_value(&doc).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 行级补丁（R 报告 P1-10）：改长文档的一行/一段，不再取全文重发全文。
    ///
    /// 何时用：doc_search 命中行号后的小修正——replace 换一段、insert 插一段、delete 删一段。
    /// 何时不用：结构性重写还是 doc_update 整体替换省事。
    pub(crate) async fn project_doc_patch(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocPatchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = Uuid::parse_str(&dp.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        // EN-226 审计补强：old_text 匹配式替换——全文唯一子串命中才执行（不限行、跨行块也行）
        if dp.mode.as_deref() == Some("replace_text") {
            let old = dp
                .anchor
                .as_deref()
                .filter(|a| !a.trim().is_empty())
                .ok_or_else(|| {
                    mcp_err(
                        ErrorCode::INVALID_PARAMS,
                        "mode=replace_text 需要 anchor 携带被替换的原文（可跨行）；content 为新文",
                    )
                })?;
            let doc = self.svc_project().get_doc(id).await.map_err(from_project)?;
            match doc.content.matches(old).count() {
                1 => {}
                0 => {
                    return Err(mcp_err(
                        ErrorCode::INVALID_PARAMS,
                        format!("old_text 零命中：{old:?}——用 doc_get 确认原文"),
                    ));
                }
                k => {
                    return Err(mcp_err(
                        ErrorCode::INVALID_PARAMS,
                        format!("old_text 命中 {k} 处——加长原文使其唯一"),
                    ));
                }
            }
            let new_full = doc
                .content
                .replacen(old, dp.content.as_deref().unwrap_or(""), 1);
            // 整文直写（update_doc COALESCE 部分更新）——不走行级 patch：
            // 行模型按 \n 切分会给尾换行文档叠出幽灵空行（审计四驳③：曾产出 "gamma\n\n"）
            let doc = self
                .svc_project()
                .update_doc(
                    id,
                    None,
                    None,
                    None,
                    Some(&new_full),
                    dp.expected_version,
                )
                .await
                .map_err(from_project)?;
            let mut v = slim_doc(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})));
            v["total_lines"] = json!(doc.content.lines().count());
            v["patched"] = json!({ "mode": "replace_text", "via_anchor": true });
            v["hint"] = json!(
                "replace_text 完成（old_text 唯一命中替换）——行号已变，继续 patch 用 anchor/anchor_end 锚点或重读 doc_get"
            );
            return ok_json(v);
        }
        // EN-226：锚点定位——anchor 按行包含匹配解析行号（唯一命中才执行），多 patch 串行不再漂移
        let (start_line, end_line, via_anchor) = if let Some(anchor) =
            dp.anchor.as_deref().filter(|a| !a.trim().is_empty())
        {
            let doc = self.svc_project().get_doc(id).await.map_err(from_project)?;
            let locate = |needle: &str| -> Result<usize, rmcp::ErrorData> {
                let hits: Vec<usize> = doc
                    .content
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.contains(needle))
                    .map(|(i, _)| i + 1)
                    .collect();
                match hits.len() {
                    1 => Ok(hits[0]),
                    0 => Err(mcp_err(
                        ErrorCode::INVALID_PARAMS,
                        format!(
                            "锚点在文档中零命中：{needle:?}——用 doc_get 确认原文措辞（锚按行包含匹配）"
                        ),
                    )),
                    n => Err(mcp_err(
                        ErrorCode::INVALID_PARAMS,
                        format!(
                            "锚点在文档中命中 {n} 行（行号 {:?}）——加长锚文本使其唯一",
                            &hits[..n.min(8)]
                        ),
                    )),
                }
            };
            let s = locate(anchor)? as i64;
            if dp.end_line.is_some() {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    "anchor 锚点模式下不收 end_line（旧行号在串行 patch 后会漂移——正是本参数要消除的问题）；多行区间用 anchor_end 锚定终点，或去掉 end_line（缺省=锚行自身）",
                ));
            }
            let e = match dp.anchor_end.as_deref().filter(|a| !a.trim().is_empty()) {
                Some(ae) => locate(ae)? as i64,
                None => s,
            };
            (s, e, true)
        } else {
            let s = dp.start_line.ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    "缺 start_line——行号定位或改用 anchor 锚点（doc_get 原文短语，按行包含匹配）",
                )
            })?;
            (s, dp.end_line.unwrap_or(s), false)
        };
        let doc = self
            .svc_project()
            .patch_doc(
                id,
                start_line,
                end_line,
                dp.mode.as_deref().unwrap_or("replace"),
                dp.content.as_deref(),
                dp.expected_version,
            )
            .await
            .map_err(from_project)?;
        let mut v = slim_doc(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})));
        v["total_lines"] = json!(doc.content.lines().count());
        v["patched"] = json!({
            "mode": dp.mode.as_deref().unwrap_or("replace"),
            "start_line": start_line,
            "end_line": end_line,
            "via_anchor": via_anchor,
        });
        v["hint"] = json!(
            "行号基于新版本——继续补丁前先重新定位（行区间读 doc_get）；串行多 patch 建议改用 anchor 锚点定位，不再受行号漂移影响"
        );
        ok_json(v)
    }

    /// 删除项目文档（不可逆）。
    ///
    /// 何时用：文档写废或彻底过时。只对明确表达的删除请求使用。
    pub(crate) async fn project_doc_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = Uuid::parse_str(&params.0.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        self.svc_project()
            .delete_doc(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.doc_id }))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_project_docs() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::project_docs_router()
}
