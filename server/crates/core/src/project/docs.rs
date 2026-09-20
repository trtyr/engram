//! `project` 的实现切片（架构治理 2026-09-21：自 project.rs 纯搬移，零行为变化）。

use super::*;

impl ProjectService {
    pub(crate) fn validate_doc_folder(folder: &str) -> Result<String, ProjectError> {
        let f = folder.trim().trim_matches('/');
        if f.is_empty() {
            return Ok(String::new());
        }
        if f.len() > 200 {
            return Err(ProjectError::BadRequest("folder 过长（>200 字符）".into()));
        }
        if f.contains('\\') || f.contains(':') {
            return Err(ProjectError::BadRequest(
                "folder 用 / 分隔；不允许反斜杠/盘符冒号".into(),
            ));
        }
        if f.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            return Err(ProjectError::BadRequest(
                "folder 含空段或 . / .. 段——用规范相对路径（如 审计、归档/ai-permissions）".into(),
            ));
        }
        Ok(f.to_string())
    }

    pub async fn add_doc(
        &self,
        project_id: Uuid,
        category: &str,
        folder: &str,
        title: &str,
        content: &str,
    ) -> Result<ProjectDocDto, ProjectError> {
        let project = self.get_project_bare(project_id).await?;
        if !project.categories.iter().any(|c| c == category) {
            return Err(Self::category_error(category, &project.categories));
        }
        let folder = Self::validate_doc_folder(folder)?;
        let id = Uuid::now_v7();
        let inserted = repo::insert_doc(
            &self.pool, id, project_id, category, &folder, title, content,
        )
        .await?;
        if inserted == 0 {
            return Err(ProjectError::Conflict(format!(
                "文档「{title}」在分类「{category}」的 folder「{folder}」下已存在——同路径 title 唯一，请 doc-update 已有文档或改 title"
            )));
        }
        self.get_doc(id).await
    }

    /// 部分更新：None 字段保持原值（并发安全——SQL 层 COALESCE，无读-改-写窗口）。
    pub async fn update_doc(
        &self,
        id: Uuid,
        category: Option<&str>,
        folder: Option<&str>,
        title: Option<&str>,
        content: Option<&str>,
        expected_version: Option<i64>,
    ) -> Result<ProjectDocDto, ProjectError> {
        // 分类只在「换到别的分类」时校验——分类被项目方移除后，存量文档仍可原地编辑
        let current = self.get_doc(id).await?;
        // 乐观锁预检（公网多Agent P001 步骤2）：带 expected_version 且与当前不符 → 409。
        // SQL 层还有原子守卫（WHERE version = $6），预检只为给出更可读的错误。
        if let Some(ev) = expected_version
            && ev != current.version
        {
            return Err(ProjectError::Conflict(format!(
                "版本冲突：文档当前 version={}，请求基于 {}——先 doc_get 取最新版本与行号再改",
                current.version, ev
            )));
        }
        if let Some(category) = category
            && category != current.category
        {
            let project = self.get_project_bare(current.project_id).await?;
            if !project.categories.iter().any(|c| c == category) {
                return Err(Self::category_error(category, &project.categories));
            }
        }
        let folder = match folder {
            Some(f) => Some(Self::validate_doc_folder(f)?),
            None => None,
        };
        let updated = repo::update_doc(
            &self.pool,
            id,
            category,
            folder.as_deref(),
            title,
            content,
            expected_version,
        )
        .await?;
        if updated == 0 {
            // 并发竞态兜底：get_doc 与 UPDATE 之间版本被改（SQL 内 version 守卫拦截）
            if expected_version.is_some() {
                return Err(ProjectError::Conflict(
                    "版本冲突：文档版本已被并发修改——先 doc_get 取最新版本再改".into(),
                ));
            }
            return Err(ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            )));
        }
        self.get_doc(id).await
    }

    pub async fn delete_doc(&self, id: Uuid) -> Result<bool, ProjectError> {
        let deleted = repo::delete_doc(&self.pool, id).await?;
        if deleted == 0 {
            return Err(ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            )));
        }
        Ok(true)
    }

    pub async fn get_doc(&self, id: Uuid) -> Result<ProjectDocDto, ProjectError> {
        repo::get_doc(&self.pool, id).await?.ok_or_else(|| {
            ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            ))
        })
    }

    /// 按行区间读文档（1-based、含两端；None = 从头/到尾）。
    /// 返回 (总行数, [(行号, 行文本)])。行号基于当前版本，改文档后需重取。
    pub async fn read_doc_lines(
        &self,
        id: Uuid,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<(i64, Vec<(i64, String)>), ProjectError> {
        let start = start.unwrap_or(1);
        let end = end.unwrap_or(i64::MAX);
        if start < 1 {
            return Err(ProjectError::BadRequest(format!(
                "start_line 从 1 开始，收到 {start}"
            )));
        }
        if start > end {
            return Err(ProjectError::BadRequest(format!(
                "start_line({start}) 不能大于 end_line({end})"
            )));
        }
        let doc = self.get_doc(id).await?;
        let total = doc.content.lines().count() as i64;
        let lines = doc
            .content
            .lines()
            .enumerate()
            .skip_while(|(i, _)| (*i as i64) < start - 1)
            .take_while(|(i, _)| (*i as i64) < end)
            .map(|(i, text)| (i as i64 + 1, text.to_string()))
            .collect();
        Ok((total, lines))
    }

    /// grep 式跨文档按行检索（大小写不敏感子串），返回命中行号 + 原文行。
    /// 定位到行号后用 read_doc_lines / project_doc_get 区间精读。
    pub async fn search_doc_lines(
        &self,
        project_id: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<DocLineHitDto>, ProjectError> {
        // 0042 检索升级：评分制多词行检索（工单「文档检索可用性」）。
        // 旧实现逐行 substring 顺序截断——零相关性排序，宽泛词首屏全泡在一篇长文里。
        let terms: Vec<String> = query
            .split_whitespace()
            .map(normalize_for_search)
            .filter(|t| !t.is_empty())
            .collect();
        if terms.is_empty() {
            return Err(ProjectError::BadRequest("检索词不能为空".to_string()));
        }
        self.get_project_bare(project_id).await?;
        let docs = repo::list_doc_contents(&self.pool, project_id).await?;

        let mut results = score_doc_hits(docs, &terms);
        let cap = limit.clamp(1, 500) as usize;

        // 排序：文档按 doc_score 降序，文档内行按行分降序 + 行号升序
        results.sort_by_key(|d| std::cmp::Reverse(d.doc_score));
        let mut hits = Vec::new();
        'outer: for mut d in results {
            d.lines.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
            let doc_hit_count = d.lines.len() as i64;
            for (line, text, _line_score) in d.lines {
                if hits.len() >= cap {
                    break 'outer;
                }
                hits.push(DocLineHitDto {
                    doc_id: d.doc_id,
                    title: d.title.clone(),
                    category: d.category.clone(),
                    line,
                    text,
                    score: d.doc_score,
                    doc_hit_count,
                });
            }
        }
        Ok(hits)
    }

    /// 行级补丁（R 报告 P1-10）：改长文档不再「取全文→重发全文」。
    /// mode=replace（默认）：[start_line, end_line]（1-based 含两端）替换为 content（必填）；
    /// mode=insert：在 start_line 行**之前**插入 content（必填），start_line 允许 total+1（追加到末尾）；
    /// mode=delete：删除 [start_line, end_line]，content 忽略。
    /// 返回更新后的文档（正文完整——调用方按需取字段）。
    pub async fn patch_doc(
        &self,
        id: Uuid,
        start_line: i64,
        end_line: i64,
        mode: &str,
        content: Option<&str>,
        expected_version: Option<i64>,
    ) -> Result<ProjectDocDto, ProjectError> {
        let doc = self.get_doc(id).await?;
        if start_line < 1 {
            return Err(ProjectError::BadRequest(format!(
                "start_line 从 1 开始（收到 {start_line}）"
            )));
        }
        // split 保留结尾空元素：原文以 \n 结尾时 split 出末尾空串，重组后 newline 语义不丢
        let mut lines: Vec<String> = doc.content.split('\n').map(str::to_string).collect();
        let real_total = lines.len() as i64; // 以 \n 结尾的文档 real_total = total + 1（末尾空串）
        let bounded_total = if doc.content.is_empty() {
            0
        } else {
            real_total
        };
        match mode {
            "replace" => {
                let Some(text) = content else {
                    return Err(ProjectError::BadRequest(
                        "mode=replace 需要传 content（替换后的文本，可多行）".into(),
                    ));
                };
                if end_line < start_line {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 不能小于 start_line({start_line})"
                    )));
                }
                if end_line > bounded_total {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 超出文档总行数（{bounded_total}）——先 doc_get 确认行号"
                    )));
                }
                let replacement: Vec<String> = text.split('\n').map(str::to_string).collect();
                let pos = (start_line - 1) as usize;
                lines.splice(pos..(end_line as usize), replacement);
            }
            "insert" => {
                let Some(text) = content else {
                    return Err(ProjectError::BadRequest(
                        "mode=insert 需要传 content（插入的文本，可多行）".into(),
                    ));
                };
                if start_line > bounded_total + 1 {
                    return Err(ProjectError::BadRequest(format!(
                        "start_line({start_line}) 超界——插入允许 1..={}（total+1 = 追加到末尾）",
                        bounded_total + 1
                    )));
                }
                let insertion: Vec<String> = text.split('\n').map(str::to_string).collect();
                let pos = (start_line - 1) as usize;
                lines.splice(pos..pos, insertion);
            }
            "delete" => {
                if end_line < start_line {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 不能小于 start_line({start_line})"
                    )));
                }
                if end_line > bounded_total {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 超出文档总行数（{bounded_total}）"
                    )));
                }
                lines.drain((start_line - 1) as usize..(end_line as usize));
            }
            other => {
                return Err(ProjectError::BadRequest(format!(
                    "mode 只支持 replace/insert/delete（收到 {other:?}）"
                )));
            }
        }
        let patched = lines.join("\n");
        self.update_doc(id, None, None, None, Some(&patched), expected_version)
            .await
    }
}

/// 文档级命中聚合（内部用）。
struct DocHits {
    doc_id: Uuid,
    title: String,
    category: String,
    doc_score: i64,
    lines: Vec<(i64, String, i64)>, // (行号, 原文, 行分)
}

/// 文档级打分：title 加权（命中 +50/词）+ 行命中文（词 +3 / 行首 +5 / 整词 +4 / 全词共现 +10）；
/// 文档分 = title 分 + 行数×5 + 行分总和。仅保留有命中的文档。
fn score_doc_hits(docs: Vec<(Uuid, String, String, String)>, terms: &[String]) -> Vec<DocHits> {
    let mut results: Vec<DocHits> = Vec::new();
    for (id, title, category, content) in docs {
        let title_norm = normalize_for_search(&title);
        // 文档级：title 命中加权（title 是文档最强信号）
        let title_score: i64 = terms
            .iter()
            .map(|t| {
                if title_norm.contains(t.as_str()) {
                    50
                } else {
                    0
                }
            })
            .sum();
        let mut lines: Vec<(i64, String, i64)> = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let line_norm = normalize_for_search(line);
            // 轻归一化版保留标点边界——整词/行首判定用
            let line_light = light_normalize(line);
            // 行分：命中词 +3/词；行首命中 +5；整词命中 +4；全词共现 +10
            let mut hit_terms = 0usize;
            let mut line_score = 0i64;
            for t in terms {
                if line_norm.contains(t.as_str()) {
                    hit_terms += 1;
                    line_score += 3;
                    if line_light.starts_with(t.as_str()) {
                        line_score += 5;
                    }
                    if is_whole_word_hit(&line_light, t) {
                        line_score += 4;
                    }
                }
            }
            if hit_terms > 0 {
                if hit_terms == terms.len() && terms.len() > 1 {
                    line_score += 10;
                }
                lines.push((i as i64 + 1, line.to_string(), line_score));
            }
        }
        if lines.is_empty() {
            continue;
        }
        // 文档分 = title 加权 + 命中密度（行数 × 5）+ 行分总和
        let doc_score =
            title_score + lines.len() as i64 * 5 + lines.iter().map(|l| l.2).sum::<i64>();
        results.push(DocHits {
            doc_id: id,
            title,
            category,
            doc_score,
            lines,
        });
    }
    results
}
