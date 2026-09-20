//! `service` 的实现切片（架构治理 2026-09-20：自 service.rs 纯搬移，零行为变化）。

use super::*;

impl WikiService {
    pub async fn lint(&self, lib: Uuid) -> Result<lint::LintReport, WikiError> {
        Ok(lint::lint(&self.pool, lib).await?)
    }

    /// 入队语义 lint（lint_deep）任务——LLM 深度检查矛盾/过时/缺页，产出入人审队列。
    pub async fn lint_deep_enqueue(
        &self,
        lib: Uuid,
        slugs: Option<Vec<String>>,
    ) -> Result<Uuid, WikiError> {
        Ok(crate::lint_deep::enqueue(&self.pool, lib, slugs).await?)
    }

    // ---------- Review ----------

    pub async fn reviews(
        &self,
        lib: Uuid,
        status: Option<&str>,
    ) -> Result<Vec<crate::review::ReviewItem>, WikiError> {
        let items = crate::review::list_by_status(&self.pool, lib, status)
            .await
            .map_err(WikiError::from)?;
        // 腐烂标注：提案指向的页面已删除 → stale 字段列出已删 slug
        crate::review::annotate_stale(&self.pool, lib, items)
            .await
            .map_err(WikiError::from)
    }

    pub async fn review_resolve(
        &self,
        id: Uuid,
        action: Option<&str>,
        dismiss: bool,
    ) -> Result<(), WikiError> {
        let hit = crate::review::resolve(&self.pool, id, action, dismiss)
            .await
            .map_err(WikiError::from)?;
        if !hit {
            // 未命中（不存在或已处理）按 404 语义返回，并给下一步指引（错误文案三问）
            return Err(WikiError::NotFound(format!(
                "review {id} 不存在或已处理——可 GET /wiki/reviews 查看当前待审列表"
            )));
        }
        Ok(())
    }

    /// Repair：lint 修而不只报（wiki 收录哲学线工单③）。
    /// 边界三级（roadmap v6）：自动做（变体死链改写 / 去链接化 / ≥3 页引用建 stub / 孤页沿出链回挂）、
    /// 留痕做（同标题重复合并——冗余丢弃或内容并入；delete_page 快照兜底 + 全库链接改指）、
    /// 不做（物理删除有内容的独立页——问用户；语义级重复发现留给 lint_deep + AI 处置）。
    /// 全程确定性（不调 LLM）；页面修改一律走 put_page 语义（版本快照 + frontmatter.via="ai"）。
    pub async fn repair(&self, lib: Uuid) -> Result<crate::repair::RepairReport, WikiError> {
        use crate::repair::{RepairAction, RepairReport};
        use std::collections::{HashMap, HashSet};

        let mut actions: Vec<RepairAction> = Vec::new();
        // 全库页快照（非 log；slug/title/content 三张 map 是本流程的工作状态）
        let pages = self.repair_pages(lib).await?;
        let checked = pages.len();
        let mut contents: HashMap<String, String> = pages
            .iter()
            .map(|(s, .., c)| (s.clone(), c.clone()))
            .collect();

        // ── 1. 同标题重复合并（留痕做，复用 merge_pages 原语）──
        actions.extend(self.repair_merge_duplicates(lib, &pages, &contents).await?);

        // merge_pages 直接落库——从库重载工作状态再进死链段
        let pages = self.repair_pages(lib).await?;
        contents = pages
            .iter()
            .map(|(s, .., c)| (s.clone(), c.clone()))
            .collect();
        let mut titles: HashMap<String, String> = pages
            .iter()
            .map(|(s, t, ..)| (s.clone(), t.clone()))
            .collect();
        let mut slugs: HashSet<String> = contents.keys().cloned().collect();

        // ── 2. 死链处理（自动做）──
        actions.extend(
            self.repair_dead_links(lib, &mut contents, &mut titles, &mut slugs)
                .await?,
        );

        // ── 3. 孤页沿出链回挂（自动做）──
        actions.extend(
            self.repair_attach_orphans(lib, &mut contents, &titles, &slugs)
                .await?,
        );
        self.rebuild_all_links(lib).await?;
        Ok(RepairReport {
            actions,
            checked_pages: checked,
        })
    }

    /// 非 log 页快照（slug / title / page_type / content）——repair 各步骤的统一取数入口。
    async fn repair_pages(
        &self,
        lib: Uuid,
    ) -> Result<Vec<(String, String, String, String)>, WikiError> {
        let pages: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type, content FROM wiki_pages \
         WHERE page_type <> 'log' AND library_id = $1 ORDER BY slug",
    )
    .bind(lib)
    .fetch_all(&self.pool)
    .await?;
        Ok(pages)
    }

    /// 步骤 1：同标题重复合并（primary = 入链最多 → 正文最长；复用 merge_pages 原语，留痕）。
    async fn repair_merge_duplicates(
        &self,
        lib: Uuid,
        pages: &[(String, String, String, String)],
        contents: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<crate::repair::RepairAction>, WikiError> {
        use crate::repair::RepairAction;
        use std::collections::HashMap;
        let mut actions: Vec<RepairAction> = Vec::new();
        let inlinks: Vec<(String, i64)> = sqlx::query_as(
            "SELECT to_slug, count(*) FROM wiki_links WHERE library_id = $1 GROUP BY to_slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let inlink_map: HashMap<String, i64> = inlinks.into_iter().collect();
        let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
        for (s, t, ..) in pages {
            by_title.entry(t.clone()).or_default().push(s.clone());
        }
        for (title, mut group) in by_title {
            if group.len() < 2 {
                continue;
            }
            group.sort_by_key(|s| {
                std::cmp::Reverse((
                    inlink_map.get(s).copied().unwrap_or(0),
                    contents.get(s).map(|c| c.chars().count()).unwrap_or(0),
                ))
            });
            let primary = group[0].clone();
            for dup in group.drain(1..) {
                let detail = self.merge_pages(lib, &primary, &dup).await?;
                actions.push(RepairAction {
                    action: "merge_duplicate".into(),
                    slug: primary.clone(),
                    detail: format!("同标题「{title}」重复合并：{detail}"),
                });
            }
        }
        Ok(actions)
    }

    /// 步骤 2：死链处理——slug 变体唯一命中则改写；≥3 页引用建 stub；否则去链接化。
    async fn repair_dead_links(
        &self,
        lib: Uuid,
        contents: &mut std::collections::HashMap<String, String>,
        titles: &mut std::collections::HashMap<String, String>,
        slugs: &mut std::collections::HashSet<String>,
    ) -> Result<Vec<crate::repair::RepairAction>, WikiError> {
        use crate::repair::RepairAction;
        use std::collections::HashMap;
        let mut actions: Vec<RepairAction> = Vec::new();
        let mut refs: HashMap<String, Vec<String>> = HashMap::new();
        for (s, c) in contents.iter() {
            for link in crate::markup::extract_wikilinks(c) {
                let (t, _) = crate::repair::split_link(&link);
                if t != *s {
                    refs.entry(t).or_default().push(s.clone());
                }
            }
        }
        let mut dead: Vec<String> = refs
            .keys()
            .filter(|t| !slugs.contains(*t) && !t.contains('/'))
            .cloned()
            .collect();
        dead.sort();
        for target in dead {
            let ref_pages: Vec<String> = refs[&target]
                .iter()
                .filter(|p| contents.contains_key(*p))
                .cloned()
                .collect();
            let n_ref = ref_pages.len();
            // a) slug 变体唯一命中 → 全部改写为真实 slug
            if let Some(action) = self
                .repair_fix_variant(lib, contents, titles, slugs, &target, &ref_pages)
                .await?
            {
                actions.push(action);
                continue;
            }
            // b) ≥3 页引用 → 建 stub（「下架不烧书」的补全起点；slug 不合法则退化为去链）
            if let Some(action) = self
                .repair_create_stub(lib, contents, titles, slugs, &target, n_ref)
                .await?
            {
                actions.push(action);
                continue;
            }
            // c) 去链接化（保留文本，摘掉链）
            actions.push(
                self.repair_delink(lib, contents, titles, &target, &ref_pages)
                    .await?,
            );
        }
        Ok(actions)
    }

    /// 死链分支 a：slug 变体唯一命中 → 把 `[[target]]` 全部改写为真实 slug（无命中返回 None）。
    async fn repair_fix_variant(
        &self,
        lib: Uuid,
        contents: &mut std::collections::HashMap<String, String>,
        titles: &std::collections::HashMap<String, String>,
        slugs: &std::collections::HashSet<String>,
        target: &str,
        ref_pages: &[String],
    ) -> Result<Option<crate::repair::RepairAction>, WikiError> {
        let sq = crate::repair::squash(target);
        let variant_matches: Vec<String> = slugs
            .iter()
            .filter(|s| crate::repair::squash(s) == sq)
            .cloned()
            .collect();
        let [real] = &variant_matches[..] else {
            return Ok(None);
        };
        let real = real.clone();
        let mut n_total = 0usize;
        for p in ref_pages {
            if let Some(c) = contents.get_mut(p) {
                let (nc, n) = crate::repair::rewrite_links(c, target, Some(&real));
                if n > 0 {
                    *c = nc;
                    n_total += n;
                    if let Some(t) = titles.get(p) {
                        self.put_page(lib, p, t, c, None, Some("ai")).await?;
                    }
                }
            }
        }
        Ok(Some(crate::repair::RepairAction {
            action: "rewrite_variant_link".into(),
            slug: real.clone(),
            detail: format!(
                "[[{target}]] 为 slug 变体，{} 页共 {n_total} 处改写为 [[{real}]]",
                ref_pages.len()
            ),
        }))
    }

    /// 死链分支 b：≥3 页引用且 slug 合法 → 建 stub（否则返回 None 交给去链接化）。
    async fn repair_create_stub(
        &self,
        lib: Uuid,
        contents: &mut std::collections::HashMap<String, String>,
        titles: &mut std::collections::HashMap<String, String>,
        slugs: &mut std::collections::HashSet<String>,
        target: &str,
        n_ref: usize,
    ) -> Result<Option<crate::repair::RepairAction>, WikiError> {
        if n_ref < 3 {
            return Ok(None);
        }
        let stub_slug = target.to_lowercase().replace(' ', "-");
        if !crate::markup::is_valid_slug(&stub_slug) {
            return Ok(None);
        }
        let content = format!(
            "# {target}\n\n（stub：repair 自动创建——{n_ref} 个页面引用指向本页但原文缺失，待补全。）"
        );
        self.put_page(lib, &stub_slug, target, &content, None, Some("ai"))
            .await?;
        slugs.insert(stub_slug.clone());
        contents.insert(stub_slug.clone(), content);
        titles.insert(stub_slug.clone(), target.to_string());
        Ok(Some(crate::repair::RepairAction {
            action: "create_stub".into(),
            slug: stub_slug,
            detail: format!("{n_ref} 个页面引用「{target}」但页面缺失——已建 stub 待补全"),
        }))
    }

    /// 死链分支 c：无匹配页面 → 去链接化（保留文本，摘掉链）。
    async fn repair_delink(
        &self,
        lib: Uuid,
        contents: &mut std::collections::HashMap<String, String>,
        titles: &std::collections::HashMap<String, String>,
        target: &str,
        ref_pages: &[String],
    ) -> Result<crate::repair::RepairAction, WikiError> {
        let mut n_total = 0usize;
        for p in ref_pages {
            if let Some(c) = contents.get_mut(p) {
                let (nc, n) = crate::repair::rewrite_links(c, target, None);
                if n > 0 {
                    *c = nc;
                    n_total += n;
                    if let Some(t) = titles.get(p) {
                        self.put_page(lib, p, t, c, None, Some("ai")).await?;
                    }
                }
            }
        }
        Ok(crate::repair::RepairAction {
            action: "delink".into(),
            slug: target.to_string(),
            detail: format!(
                "[[{target}]] 无匹配页面且仅 {} 页引用——已去链接化（{n_total} 处）",
                ref_pages.len()
            ),
        })
    }

    /// 步骤 3：孤页沿出链回挂（纯增益：不改不删，只加一行「相关」链接）。
    async fn repair_attach_orphans(
        &self,
        lib: Uuid,
        contents: &mut std::collections::HashMap<String, String>,
        titles: &std::collections::HashMap<String, String>,
        slugs: &std::collections::HashSet<String>,
    ) -> Result<Vec<crate::repair::RepairAction>, WikiError> {
        use crate::repair::RepairAction;
        let mut actions: Vec<RepairAction> = Vec::new();
        // ── 3. 孤页沿出链回挂（自动做）──
        // 先统一重建 wiki_links（合并/改写后的真实出链），再找 0 入链页，
        // 把孤页回挂到它第一个「目标存在的库内出链」页的相关区（纯增益：不改不删只加一行）。
        self.rebuild_all_links(lib).await?;
        let system = ["index", "log", "overview"];
        let orphans: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.slug, p.content FROM wiki_pages p \
         WHERE p.page_type <> 'log' AND p.library_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM wiki_links l WHERE l.library_id = $1 AND l.to_slug = p.slug)",
    )
    .bind(lib)
    .fetch_all(&self.pool)
    .await?;
        for (oslug, ocontent) in orphans {
            if system.contains(&oslug.as_str()) || !slugs.contains(&oslug) {
                continue;
            }
            let mut target: Option<String> = None;
            for link in crate::markup::extract_wikilinks(&ocontent) {
                let (t, _) = crate::repair::split_link(&link);
                if t == oslug || !slugs.contains(&t) {
                    continue;
                }
                let exists: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM wiki_pages WHERE slug = $1 AND library_id = $2",
                )
                .bind(&t)
                .bind(lib)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);
                if exists > 0 {
                    target = Some(t);
                    break;
                }
            }
            let Some(target) = target else {
                continue;
            };
            let Some(tc) = contents.get_mut(&target) else {
                continue;
            };
            if tc.contains(&format!("[[{oslug}]]")) {
                continue; // 已有链接，不重复挂
            }
            tc.push_str(&format!("\n\n相关：[[{oslug}]]"));
            if let Some(t) = titles.get(&target) {
                self.put_page(lib, &target, t, tc, None, Some("ai")).await?;
            }
            actions.push(RepairAction {
                action: "attach_orphan".into(),
                slug: oslug.clone(),
                detail: format!("孤页无入链——已回挂到其出链目标「{target}」的相关区"),
            });
        }
        Ok(actions)
    }

    // ---------- 版本历史（R 报告建议 #5：列表 + 回滚；快照按 (library_id, slug) 隔离） ----------

    /// 裁剪旧快照（每 slug 只留最近 VERSION_KEEP 条；best-effort，不影响主流程）。
    pub(super) async fn prune(&self, lib: Uuid, slug: &str) {
        prune_page_versions(&self.pool, lib, slug, VERSION_KEEP).await;
    }

    // ---------- 级联删除 ----------

    pub async fn delete_source_cascade(
        &self,
        lib: Uuid,
        source_id: Uuid,
    ) -> Result<crate::cascade::CascadeReport, WikiError> {
        // 源存在性前置检查（NotFound 语义）：按 (library_id, id) 匹配——
        // 源不存在或属于其他库都按 404 处理（多库隔离）
        let hit: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM wiki_sources WHERE id = $1 AND library_id = $2")
                .bind(source_id)
                .bind(lib)
                .fetch_optional(&self.pool)
                .await?;
        if hit.is_none() {
            return Err(WikiError::NotFound(format!("源 {source_id} 不存在")));
        }
        // W-16（2026-09-04）：先取消该源在途的织入任务——否则级联删完，
        // 队列里 analyze/generate 继续跑，边删边产页（测试实测页面 81→84）。
        // running 中的任务若已过写库点仍可能落页，残留由 stale_source lint 报出。
        sqlx::query(
            "UPDATE jobs SET status = 'cancelled', error = '源已删除——织入任务随级联取消', \
         locked_by = NULL, locked_at = NULL \
         WHERE kind IN ('wiki_analyze','wiki_generate') \
         AND status IN ('pending','running') \
         AND payload->>'source_id' = $1::text",
        )
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        let report = crate::cascade::cascade_delete_source(&self.pool, source_id)
            .await
            .map_err(WikiError::from)?;
        // 腐烂治理（工单「人审队列腐烂」）：指向该源的 open 提案自动 dismissed（可审计不删数据）
        let _ = crate::review::cascade_dismiss(&self.pool, lib, None, Some(source_id)).await; // 有意忽略：派生审查项清理 best-effort
        // 破坏性操作落审计行（与 memory 域「job 行即审计链」同哲学）——best-effort，不阻断返回
        self.audit(
            "wiki_source_cascade_delete",
            serde_json::json!({ "source_id": source_id, "report": report }),
        )
        .await;
        Ok(report)
    }

    /// 供 API 列出可删的 sources（库内）。
    pub async fn list_sources(
        &self,
        lib: Uuid,
    ) -> Result<Vec<(Uuid, Option<String>, String, String)>, WikiError> {
        Ok(sqlx::query_as(
            "SELECT id, title, status, sha256 FROM wiki_sources \
         WHERE library_id = $1 ORDER BY created_at DESC",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 审计行（清空不吞审计凭证）：破坏性操作落 jobs 成功行，best-effort。
    /// jobs 表不挂库——审计链全库共享，故本方法不引入 lib 参数。
    pub async fn audit(&self, kind: &str, payload: serde_json::Value) {
        sqlx::query(
            "INSERT INTO jobs (id, kind, payload, status, attempts, max_attempts, \
         progress, started_at, finished_at) \
         VALUES ($1, $2, $3, 'succeeded', 1, 1, $3, now(), now())",
        )
        .bind(Uuid::now_v7())
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await
        .ok();
    }
}
