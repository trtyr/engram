//! `service` 的实现切片（架构治理 2026-09-20：自 service.rs 纯搬移，零行为变化）。

use super::*;

impl WikiService {
    /// 页面列表（D28 keyset 分页）：cursor = 上一页最后一条的
    /// `{updated_at ISO8601}|{id}`，首查不传。ORDER BY 带 id 决稳——
    /// 此前静默截断曾让最老的页面从列表「消失」（graph/lint 却可见）。
    /// 单库终局（2026-09-20）：limit=None = 全量返回（用户拍板不要上限）——
    /// 原 .min(300) 硬上限随多库时代一起拆除。
    /// 只列指定库的页面（slug 跨库可重名）。
    pub async fn list_pages(
        &self,
        lib: Uuid,
        page_type: Option<&str>,
        folder: Option<&str>,
        limit: Option<i64>,
        cursor: Option<&str>,
    ) -> Result<Vec<WikiPageMetaDto>, WikiError> {
        let parse_cursor =
            |raw: &str| -> Result<(chrono::DateTime<chrono::Utc>, uuid::Uuid), WikiError> {
                let parts: Vec<&str> = raw.split('|').collect();
                if parts.len() != 2 {
                    return Err(WikiError::BadRequest(format!(
                        "cursor 非法（收到 {raw:?}）——期望 {{updated_at ISO8601}}|{{id}}，取上一页最后一条构造"
                    )));
                }
                let ts = chrono::DateTime::parse_from_rfc3339(parts[0].trim())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .map_err(|_| {
                        WikiError::BadRequest(format!(
                            "cursor 时间无法解析（收到 {:?}）——期望 ISO8601",
                            parts[0]
                        ))
                    })?;
                let id = uuid::Uuid::parse_str(parts[1].trim()).map_err(|_| {
                    WikiError::BadRequest(format!("cursor id 不是合法 UUID（收到 {:?}）", parts[1]))
                })?;
                Ok((ts, id))
            };
        // 规模化（2026-09-20，用户拍板「加载太慢」）：列表**不回正文**——万页下每页 content
        // 合计 18MB，而目录树只用元数据（正文走 GET /wiki/pages/{slug}）；folder 支持按子树
        // 拉取（folder = $4 或 folder LIKE $4 || '/%'），配合 list_folders 做前端懒加载。
        const COLS: &str = "SELECT id, slug, title, page_type, folder, frontmatter, origin, version, \
                        updated_at, char_length(content) AS content_chars FROM wiki_pages";
        match cursor {
            None | Some("") => {
                let sql = format!(
                    "{COLS} WHERE library_id = $1 AND ($2::text IS NULL OR page_type = $2) \
                   AND ($4::text IS NULL OR folder = $4 OR folder LIKE $4 || '/%') \
                   AND page_type NOT IN ('log') \
                 ORDER BY updated_at DESC, id DESC LIMIT $3"
                );
                Ok(sqlx::query_as::<_, WikiPageMetaDto>(&sql)
                    .bind(lib)
                    .bind(page_type)
                    .bind(limit)
                    .bind(folder)
                    .fetch_all(&self.pool)
                    .await?)
            }
            Some(raw) => {
                let (ts, id) = parse_cursor(raw)?;
                let sql = format!(
                    "{COLS} WHERE library_id = $1 AND ($2::text IS NULL OR page_type = $2) \
                   AND ($4::text IS NULL OR folder = $4 OR folder LIKE $4 || '/%') \
                   AND page_type NOT IN ('log') \
                   AND (updated_at, id) < ($5::timestamptz, $6::uuid) \
                 ORDER BY updated_at DESC, id DESC LIMIT $3"
                );
                Ok(sqlx::query_as::<_, WikiPageMetaDto>(&sql)
                    .bind(lib)
                    .bind(page_type)
                    .bind(limit)
                    .bind(folder)
                    .bind(ts)
                    .bind(id)
                    .fetch_all(&self.pool)
                    .await?)
            }
        }
    }

    /// 目录骨架索引（规模化 2026-09-20）：只回 folder 路径与页数——供前端懒加载树渲染
    /// 空文件夹与页数角标。万页下这是「folder 数量级」行数（几百），不是一万行页面。
    pub async fn list_folders(&self, lib: Uuid) -> Result<Vec<(String, i64)>, WikiError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT folder, count(*) FROM wiki_pages \
         WHERE library_id = $1 AND page_type NOT IN ('log') \
         GROUP BY folder ORDER BY folder",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 读单页（库内）。先精确匹配；未中则按「小写 + 空格转连字符」宽容重查——LLM 生成正文时
    /// 常把双链写成标题原文（[[Rust 异步运行时]]），与真实 slug（rust-异步运行时）
    /// 只差大小写和分隔符，精确匹配 404 后点过去就"没反应"。
    /// R 报告 P1-11 双寻址：slug 未中再按 title 精确兜底（标题寻址）。
    pub async fn get_page(&self, lib: Uuid, slug: &str) -> Result<WikiPageDto, WikiError> {
        sqlx::query_as::<_, WikiPageDto>(
            "SELECT * FROM wiki_pages \
         WHERE (slug = $1 OR slug = lower(replace($1, ' ', '-')) OR title = $1) \
           AND library_id = $2 \
         ORDER BY (slug = $1) DESC, (title = $1) DESC LIMIT 1",
        )
        .bind(slug)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| WikiError::NotFound(format!("页面 {slug} 不存在")))
    }

    /// slug/title 宽容解析成真实 slug（删除/版本操作用，与 get_page 同一匹配口径；库内）。
    pub(super) async fn resolve_slug(
        &self,
        lib: Uuid,
        slug_or_title: &str,
    ) -> Result<String, WikiError> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT slug FROM wiki_pages \
         WHERE (slug = $1 OR slug = lower(replace($1, ' ', '-')) OR title = $1) \
           AND library_id = $2 \
         ORDER BY (slug = $1) DESC LIMIT 1",
        )
        .bind(slug_or_title)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| WikiError::NotFound(format!("页面 {slug_or_title} 不存在")))
    }

    /// 人工编辑：origin=human、版本递增、重嵌入。folder 可选（None=保持原值/默认空）。
    /// via 可选（S-7）：执行者标记（如 "ai"）——落 frontmatter.via，区分真人编辑与 AI 代执行。
    /// 多库：快照/页面/链接全部按 (library_id, slug) 操作。
    pub async fn put_page(
        &self,
        lib: Uuid,
        slug: &str,
        title: &str,
        content: &str,
        folder: Option<&str>,
        via: Option<&str>,
    ) -> Result<WikiPageDto, WikiError> {
        if !crate::markup::is_valid_slug(slug) {
            return Err(WikiError::BadRequest(
                "slug 非法：仅允许字母/数字/-/_/·，≤80 字符，不含空格与路径分隔符".into(),
            ));
        }
        // 新建时的 frontmatter：title/sources + via（若有）
        let mut fm_insert = serde_json::json!({"title": title, "sources": []});
        if let Some(v) = via {
            fm_insert["via"] = serde_json::json!(v);
        }
        // 更新时的 frontmatter 合并块：title 恒同步（graph 节点标题依赖，D8）+
        // via（若有）——|| 合并只覆盖指定键，sources 等其余键保留
        let mut fm_merge_obj = serde_json::json!({ "title": title });
        if let Some(v) = via {
            fm_merge_obj["via"] = serde_json::json!(v);
        }
        let fm_merge = fm_merge_obj.to_string();
        // 版本历史（R 报告建议 #5）：覆盖前先把现状快照进 wiki_page_versions——
        // 此前 version 只是计数器，覆盖即失忆。INSERT..SELECT 天然幂等（无旧页 0 行）。
        sqlx::query(
        "INSERT INTO wiki_page_versions (id, library_id, slug, version, title, page_type, folder, content, origin) \
         SELECT $1, $3, slug, version, title, page_type, folder, content, origin \
         FROM wiki_pages WHERE slug = $2 AND library_id = $3",
    )
    .bind(Uuid::now_v7())
    .bind(slug)
    .bind(lib)
    .execute(&self.pool)
    .await?;
        self.prune(lib, slug).await;
        let row = sqlx::query_as::<_, WikiPageDto>(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
         VALUES ($1, $2, $3, $4, 'concept', COALESCE($5, ''), $6, $7::jsonb, 'human', 1, to_tsvector('simple', $8)) \
         ON CONFLICT (library_id, slug) DO UPDATE SET \
            title = $4, content = $6, origin = 'human', \
            folder = COALESCE($5, wiki_pages.folder), \
            frontmatter = wiki_pages.frontmatter || $9::jsonb, \
            version = wiki_pages.version + 1, updated_at = now(), \
            tsv = to_tsvector('simple', $8) \
         RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(lib)
    .bind(slug)
    .bind(title)
    .bind(folder)
    .bind(content)
    .bind(fm_insert.to_string())
    .bind(page_tsv_text(slug, title, content))
    .bind(fm_merge)
    .fetch_one(&self.pool)
    .await?;

        // D4：落页后重算本页 wikilinks——graph/孤页检测与 lint 同源（此前
        // put_page 不写 wiki_links，AI 写页的互链对图与 lint 不可见）
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1 AND library_id = $2")
            .bind(slug)
            .bind(lib)
            .execute(&self.pool)
            .await?;
        let mut cross_targets: Vec<(String, String)> = Vec::new();
        for target in crate::markup::extract_wikilinks(content) {
            // R 多库补全：跨库引用 [[lib/slug]] 走 wiki_cross_links（校验目标存在后建链）
            if let Some((to_lib, to_slug)) = crate::markup::split_cross_lib(&target) {
                cross_targets.push((to_lib, to_slug));
                continue;
            }
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
             VALUES ($3, $1, $2, 3.0) \
             ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(&target)
            .bind(lib)
            .execute(&self.pool)
            .await
            .ok();
        }
        crate::cross_links::sync_page(&self.pool, lib, slug, &cross_targets).await?;
        Ok(row)
    }

    /// 删除页面（D10：MCP wiki_delete_page / HTTP DELETE /wiki/pages/{slug}）——
    /// 连带清理双向 wikilinks（图与孤页检测不留幽灵边）。
    /// 删除前快照最后状态进 wiki_page_versions——误删可经 restore_version 重建
    /// （R 报告「删除抹掉全部历史」的回收通道；版本历史本身保留）。库内操作。
    pub async fn delete_page(&self, lib: Uuid, slug: &str) -> Result<bool, WikiError> {
        let slug = self.resolve_slug(lib, slug).await?;
        sqlx::query(
        "INSERT INTO wiki_page_versions (id, library_id, slug, version, title, page_type, folder, content, origin) \
         SELECT $1, $3, slug, version, title, page_type, folder, content, origin \
         FROM wiki_pages WHERE slug = $2 AND library_id = $3",
    )
    .bind(Uuid::now_v7())
    .bind(&slug)
    .bind(lib)
    .execute(&self.pool)
    .await?;
        self.prune(lib, &slug).await;
        let n = sqlx::query("DELETE FROM wiki_pages WHERE slug = $1 AND library_id = $2")
            .bind(&slug)
            .bind(lib)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(WikiError::NotFound(format!("页面 {slug} 不存在")));
        }
        sqlx::query(
            "DELETE FROM wiki_links WHERE (from_slug = $1 OR to_slug = $1) AND library_id = $2",
        )
        .bind(&slug)
        .bind(lib)
        .execute(&self.pool)
        .await?;
        // R 多库补全：跨库引用级联清理（from 侧与 to 侧）
        crate::cross_links::delete_page_cleanup(&self.pool, lib, &slug).await?;
        // 腐烂治理（工单「人审队列腐烂」）：指向该页的 open 提案自动 dismissed（可审计不删数据）
        let _ = crate::review::cascade_dismiss(&self.pool, lib, Some(&slug), None).await; // 有意忽略：派生审查项清理 best-effort
        Ok(true)
    }

    pub async fn merge_pages(
        &self,
        lib: Uuid,
        primary_slug: &str,
        duplicate_slug: &str,
    ) -> Result<String, WikiError> {
        if primary_slug == duplicate_slug {
            return Err(WikiError::BadRequest("主页面与重复页面不能是同一页".into()));
        }
        let primary = self.resolve_slug(lib, primary_slug).await?;
        let dup = self.resolve_slug(lib, duplicate_slug).await?;
        let dup_page = self.get_page(lib, &dup).await?;
        let pri_page = self.get_page(lib, &primary).await?;

        let (mut new_primary, discarded) = merge_content(&pri_page, &dup_page, &dup);
        let rewrite_total = self.rewrite_referencing_links(lib, &dup, &primary).await?;

        // 3) primary 落合并内容（内容有变才写）
        if new_primary != pri_page.content {
            self.put_page(
                lib,
                &primary,
                &pri_page.title,
                &new_primary,
                None,
                Some("ai"),
            )
            .await?;
        }
        self.delete_page(lib, &dup).await?;
        self.rebuild_all_links(lib).await?;
        Ok(format!(
            "{dup} → {primary}（{}，链接改写 {rewrite_total} 处）",
            if discarded {
                "冗余丢弃"
            } else {
                "内容并入"
            }
        ))
    }

    /// 全库其他页指向 `dup` 的链接改指 `primary`（防合并后新增死链），返回改写页数。
    async fn rewrite_referencing_links(
        &self,
        lib: Uuid,
        dup: &str,
        primary: &str,
    ) -> Result<usize, WikiError> {
        // 2) 全库其他页指向 dup 的链接改指 primary（防合并后新增死链）
        let mut rewrite_total = 0usize;
        let others: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, content FROM wiki_pages WHERE library_id = $1 AND slug <> $2 AND slug <> $3",
    )
    .bind(lib)
    .bind(dup)
    .bind(primary)
    .fetch_all(&self.pool)
    .await?;
        for (slug, content) in &others {
            let (nc, n) = crate::repair::rewrite_links(content, dup, Some(primary));
            if n > 0 {
                rewrite_total += n;
                let title: String = sqlx::query_scalar(
                "SELECT COALESCE(frontmatter->>'title', slug) FROM wiki_pages WHERE slug = $1 AND library_id = $2",
            )
            .bind(slug)
            .bind(lib)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|_| slug.clone());
                self.put_page(lib, slug, &title, &nc, None, Some("ai"))
                    .await?;
            }
        }
        Ok(rewrite_total)
    }

    /// 页面版本列表（新→旧；不带正文，content_chars 供决策）。
    /// 已删除的页面按 slug 直查快照表——恢复通道不因页面不在而 404。
    pub async fn page_versions(
        &self,
        lib: Uuid,
        slug: &str,
    ) -> Result<Vec<WikiPageVersionDto>, WikiError> {
        let slug = match self.resolve_slug(lib, slug).await {
            Ok(s) => s,
            Err(WikiError::NotFound(_)) => slug.to_string(),
            Err(e) => return Err(e),
        };
        Ok(sqlx::query_as::<_, WikiPageVersionDto>(
            "SELECT id, slug, version, title, page_type, folder, origin, \
         length(content)::bigint AS content_chars, created_at \
         FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 \
         ORDER BY version DESC, created_at DESC",
        )
        .bind(&slug)
        .bind(lib)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 读取一个版本快照的正文（回滚前预览用）。已删除页面按 slug 直查。
    pub async fn page_version_content(
        &self,
        lib: Uuid,
        slug: &str,
        version: i32,
    ) -> Result<String, WikiError> {
        let slug = match self.resolve_slug(lib, slug).await {
            Ok(s) => s,
            Err(WikiError::NotFound(_)) => slug.to_string(),
            Err(e) => return Err(e),
        };
        let row: Option<String> = sqlx::query_scalar(
            "SELECT content FROM wiki_page_versions \
         WHERE slug = $1 AND library_id = $2 AND version = $3",
        )
        .bind(&slug)
        .bind(lib)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| {
            WikiError::NotFound(format!(
                "页面 {slug} 没有版本 {version}——先查 versions 列表取可用版本号"
            ))
        })
    }

    /// 回滚到某个版本快照：以「当前版本 +1」落地（历史不可变，回滚也是新版本）。
    /// 页面已被删除时从快照重建（沿用页型/目录，版本号接续快照史）。库内操作。
    pub async fn restore_page_version(
        &self,
        lib: Uuid,
        slug: &str,
        version: i32,
    ) -> Result<WikiPageDto, WikiError> {
        let snap: (String, String, String, String, i32) = sqlx::query_as(
            "SELECT title, content, page_type, folder, version \
         FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 AND version = $3",
        )
        .bind(slug)
        .bind(lib)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            WikiError::NotFound(format!(
                "没有 {slug}#{version} 的快照——先查 versions 列表取可用版本号"
            ))
        })?;
        let (title, content, page_type, folder, _) = snap;
        match self.get_page(lib, slug).await {
            Ok(_) => {
                // 活页：走 put_page（快照现状 → 落目标内容 → 版本 +1、重算链接）
                self.put_page(lib, slug, &title, &content, Some(&folder), Some("restore"))
                    .await
            }
            Err(WikiError::NotFound(_)) => {
                // 死页重建：版本号接续快照史（避免清零后与历史快照版本撞号）
                let next: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(version), 0) + 1 \
                 FROM (SELECT version FROM wiki_page_versions WHERE slug = $1 AND library_id = $2 \
                       UNION ALL SELECT version FROM wiki_pages WHERE slug = $1 AND library_id = $2) t",
            )
            .bind(slug)
            .bind(lib)
            .fetch_one(&self.pool)
            .await?;
                let fm = serde_json::json!({"title": title, "sources": [], "via": "restore"});
                let row = sqlx::query_as::<_, WikiPageDto>(
                "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, 'human', $9, to_tsvector('simple', $10)) \
                 RETURNING *",
            )
            .bind(Uuid::now_v7())
            .bind(lib)
            .bind(slug)
            .bind(&title)
            .bind(&page_type)
            .bind(&folder)
            .bind(&content)
            .bind(fm.to_string())
            .bind(next)
            .bind(page_tsv_text(slug, &title, &content))
            .fetch_one(&self.pool)
            .await?;
                for target in crate::markup::extract_wikilinks(&content) {
                    if crate::markup::split_cross_lib(&target).is_some() {
                        continue; // 跨库引用不进库内 wiki_links
                    }
                    sqlx::query(
                        "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                     VALUES ($3, $1, $2, 3.0) \
                     ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
                    )
                    .bind(slug)
                    .bind(&target)
                    .bind(lib)
                    .execute(&self.pool)
                    .await
                    .ok();
                }
                Ok(row)
            }
            Err(e) => Err(e),
        }
    }
}

/// 1) 内容并入（冗余丢弃 / append 章节）；primary 自身引用 dup → 去链接化。
fn merge_content(
    pri: &crate::WikiPageDto,
    dup_page: &crate::WikiPageDto,
    dup: &str,
) -> (String, bool) {
    // 1) 内容并入（冗余丢弃 / append 章节）；primary 自身引用 dup → 去链接化
    let dup_c = dup_page.content.trim();
    let discarded = dup_c.is_empty() || pri.content.contains(dup_c);
    let mut new_primary = if discarded {
        pri.content.clone()
    } else {
        format!(
            "{}\n\n## 合并自〈{}〉（{dup}）\n\n{}",
            pri.content, dup_page.title, dup_c
        )
    };
    let (_, n_self) = crate::repair::rewrite_links(&new_primary, &dup, None);
    if n_self > 0 {
        let (nc, _) = crate::repair::rewrite_links(&new_primary, &dup, None);
        new_primary = nc;
    }
    (new_primary, discarded)
}
