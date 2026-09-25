//! `service` 的实现切片（架构治理 2026-09-20：自 service.rs 纯搬移，零行为变化）。

use super::*;

impl WikiService {
    /// 审计缺陷④：存量页向量回填（cap 50/次）——repair job 与织入尾部的自愈入口。
    pub async fn backfill_embeddings(&self, lib: Uuid) -> Result<usize, WikiError> {
        backfill_page_embeddings(&self.pool, self.llm.as_ref(), lib).await
    }

    /// 触发两步 ingest（文本 + 标题）。返回三态（D27）：已就绪跳过 / 在途 / 新入队。
    /// D24：空标题/空文本响亮拒绝（空文本任务曾在队列里滞留不执行、空标题白烧一次 LLM）。
    /// 多库：原料落到指定库（sha 去重也只在库内生效）。
    pub async fn ingest(
        &self,
        lib: Uuid,
        title: &str,
        text: &str,
    ) -> Result<crate::ingest::IngestOutcome, WikiError> {
        if title.trim().is_empty() {
            return Err(WikiError::BadRequest(
                "title 不能为空——织入来源需要可辨认的标题".into(),
            ));
        }
        if text.trim().is_empty() {
            return Err(WikiError::BadRequest(
                "text 不能为空——空文本织入只会浪费 LLM 调用".into(),
            ));
        }
        // 收录判据③（2026-09-19 wiki 收录哲学线，用户拍板）：原料须有实质内容才进织入。
        // 种子日志/单行注记（一行日期+短语）织入只会产出无根页面并污染复核队列。
        const MIN_INGEST_CHARS: usize = 80;
        let trimmed = text.trim();
        if trimmed.chars().count() < MIN_INGEST_CHARS {
            return Err(WikiError::BadRequest(format!(
                "原料正文仅 {} 字，低于织入门槛 {MIN_INGEST_CHARS} 字——种子日志/单行注记不属于知识原料（收录判据③：原料须有实质内容）。如是真实知识请补全正文后再织入",
                trimmed.chars().count()
            )));
        }
        Ok(ingest::enqueue_ingest(&self.queue, lib, title, text).await?)
    }

    /// 从 wiki 文档触发织入（upload 与 URL 通用，2026-09-04 补 URL 兜底）：
    /// raw_path 有 → 重新读取原文件并解析（保留原行为）；
    /// raw_path 空（URL 摄取）→ 用已分块文本按 seq 拼接——此前 URL 文档既不能
    /// --doc-id 手动织入（404）也不会被自动织入静默跳过，两路都收敛到 ingest(title, text)。
    /// 多库：文档按 (id, library_id) 匹配——跨库文档按不存在处理。
    pub async fn ingest_document(
        &self,
        lib: Uuid,
        doc_id: Uuid,
    ) -> Result<crate::ingest::IngestOutcome, WikiError> {
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT title, raw_path, mime FROM wiki_documents WHERE id = $1 AND library_id = $2",
        )
        .bind(doc_id)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?;
        let Some((title, raw_path, mime)) = row else {
            return Err(WikiError::NotFound(format!("文档 {doc_id} 不存在")));
        };
        let text = if raw_path.as_deref().is_some_and(|p| !p.is_empty()) {
            let path = raw_path.unwrap_or_default();
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| WikiError::BadRequest(format!("读文件失败: {e}")))?;
            engram_parsing::parse_bytes(&name, mime.as_deref(), &bytes)
                .map_err(|e| WikiError::BadRequest(e.to_string()))?
        } else {
            // URL 摄取：无本地文件，用 chunks 表已解析文本按序拼接（库内）
            let chunks: Vec<String> = sqlx::query_scalar(
                "SELECT c.content FROM wiki_chunks c \
             WHERE c.document_id = $1 AND c.library_id = $2 ORDER BY c.seq",
            )
            .bind(doc_id)
            .bind(lib)
            .fetch_all(&self.pool)
            .await?;
            if chunks.is_empty() {
                return Err(WikiError::BadRequest(format!(
                    "文档 {doc_id} 无本地文件且无可织入的分块（可能尚未解析完成）"
                )));
            }
            chunks.join("\n\n")
        };
        self.ingest(lib, &title, &text).await
    }

    /// 内容目录（karpathy LLM Wiki 的 index 页等价物）：按 page_type 分组的动态聚合，
    /// 只读不落库。每页含入链数与首段摘要——人读与 LLM 导航双用途。
    pub async fn index(&self, lib: Uuid) -> Result<serde_json::Value, WikiError> {
        let rows: Vec<(String, String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT p.page_type, p.slug, p.title, \
                COALESCE(split_part(left(regexp_replace(p.content, E'[\\n\\r]+', ' ', 'g'), 160), '。', 1), '') AS summary, \
                (SELECT count(*)::bigint FROM wiki_links l WHERE l.to_slug = p.slug AND l.library_id = p.library_id) AS inlinks \
         FROM wiki_pages p WHERE p.library_id = $1 AND p.page_type <> 'log' \
         ORDER BY p.page_type, p.slug",
    )
    .bind(lib)
    .fetch_all(&self.pool)
    .await?;
        let mut groups: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
            std::collections::BTreeMap::new();
        for (page_type, slug, title, summary, inlinks) in rows {
            groups
                .entry(page_type)
                .or_default()
                .push(serde_json::json!({
                    "slug": slug, "title": title,
                    "summary": summary, "inlinks": inlinks.unwrap_or(0),
                }));
        }
        let pages: serde_json::Map<String, serde_json::Value> = groups
            .into_iter()
            .map(|(k, v)| (k, serde_json::json!(v)))
            .collect();
        Ok(serde_json::json!({ "groups": pages }))
    }

    /// 问答/分析产物归档（karpathy LLM Wiki：好答案不该消失在聊天记录里）——
    /// 以 page_type=analysis 落页（0040）（复用 put_page 的版本快照与 wikilinks 重算），
    /// 再对 related 页面补双向链接（归档页 ↔ 相关页）。
    pub async fn archive_answer(
        &self,
        lib: Uuid,
        slug: &str,
        title: &str,
        content: &str,
        related: &[String],
    ) -> Result<WikiPageDto, WikiError> {
        if !crate::markup::is_valid_slug(slug) {
            return Err(WikiError::BadRequest(
                "slug 非法：仅允许字母/数字/-/_/·，≤80 字符，不含空格与路径分隔符".into(),
            ));
        }
        let mut page = self
            .put_page(lib, slug, title, content, None, Some("archive"))
            .await?;
        // 归档页固定为 analysis 类型（put_page 硬编码 concept，这里矫正；analysis 由 0040 加入 CHECK）
        sqlx::query("UPDATE wiki_pages SET page_type = 'analysis' WHERE id = $1")
            .bind(page.id)
            .execute(&self.pool)
            .await?;
        page.page_type = "analysis".into();
        // related 双向链接（归档页 ↔ 相关页；目标不存在时跳过该条——与 wikilink 死链语义一致，由 lint 报告）
        for target in related {
            if target == slug {
                continue;
            }
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
             VALUES ($3, $1, $2, 3.0) ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(target)
            .bind(lib)
            .execute(&self.pool)
            .await
            .ok();
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
             VALUES ($3, $1, $2, 3.0) ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(target)
            .bind(slug)
            .bind(lib)
            .execute(&self.pool)
            .await
            .ok();
        }
        Ok(page)
    }

    /// 存量页 tsv 重刷（EN-63）：内容页、slug+title+content、wiki 分词变体。
    ///
    /// 排除 index/log/overview 系统页——它们是目录/日志结构页不是内容（insights/lint/
    /// cascade 等全部读者都排除它们），且 overview 页聚合了几乎全库正文、是关键词汤；
    /// 历史上它们 tsv 为 NULL 不参与 FTS，重刷若包含会让系统页霸榜（audit 实证回归）。
    /// 幂等（值不变不写）；jieba 分词必须经 Rust，故逐页计算。
    pub async fn backfill_tsv(&self, lib: Uuid) -> Result<u64, WikiError> {
        // 历史残留清理：audit 前的重刷（无排除版）或任何途径给系统页写过的 tsv 必须清 NULL，
        // 否则它们继续参与 FTS 霸榜——仅「不更新」不够
        sqlx::query(
            "UPDATE wiki_pages SET tsv = NULL \
         WHERE library_id = $1 AND page_type IN ('index','log','overview') AND tsv IS NOT NULL",
        )
        .bind(lib)
        .execute(&self.pool)
        .await?;
        let pages: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages \
         WHERE library_id = $1 AND page_type NOT IN ('index','log','overview')",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let mut n = 0u64;
        for (slug, title, content) in &pages {
            let text = page_tsv_text(slug, title, content);
            let r = sqlx::query(
                "UPDATE wiki_pages SET tsv = to_tsvector('simple', $3) \
             WHERE slug = $1 AND library_id = $2 \
               AND tsv IS DISTINCT FROM to_tsvector('simple', $3)",
            )
            .bind(slug)
            .bind(lib)
            .bind(&text)
            .execute(&self.pool)
            .await?;
            n += r.rows_affected();
        }
        Ok(n)
    }

    /// 死任务重跑（UI 辅助）。
    pub async fn reingest(&self, source_id: Uuid) -> Result<(), WikiError> {
        self.queue
            .enqueue(
                JobTemplate::new("wiki_analyze")
                    .with_payload(serde_json::json!({"source_id": source_id})),
            )
            .await?;
        Ok(())
    }

    // ---------- queries 页型闭环 ----------

    /// 检索结果/问答 → 直接落 queries 页型（人工归档）→ 同时入队再摄取吸收实体概念（库内）。
    pub async fn archive_query(
        &self,
        lib: Uuid,
        title: &str,
        question: &str,
        answer: &str,
    ) -> Result<(bool, String), WikiError> {
        // W-13（2026-09-04）：同 title 已存档 → 幂等跳过（契约「重复→skipped」），
        // 不再落页 version+1 + 再摄取烧 LLM。
        // EN-243③：slug 遵守自家规范（is_valid_slug：禁空白，≤80）——空白折叠为连字符
        let compact: String = title.split_whitespace().collect::<Vec<_>>().join("-");
        let slug: String = format!("query-{compact}").chars().take(80).collect();
        // D5：slug 或 title 任一命中即幂等跳过（此前仅 slug 检查，title 尾随差异漏网 → 覆盖旧答案）
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM wiki_pages WHERE (slug = $1 OR title = $2) AND library_id = $3",
        )
        .bind(&slug)
        .bind(title)
        .bind(lib)
        .fetch_optional(&self.pool)
        .await?;
        if exists.is_some() {
            return Ok((true, slug));
        }
        let ts = chrono::Utc::now().format("%Y-%m-%d");
        let content = format!(
            "# {title}\n\n**问**：{question}\n\n**答**：{answer}\n\n（来源：检索存档 {ts}）"
        );

        // 1) 直接落 queries 页（page_type=queries，origin=human——人触发的存档）
        let fm = serde_json::json!({"title": title, "page_type": "queries", "sources": []});
        sqlx::query(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
         VALUES ($1, $2, $3, $4, 'queries', '查询', $5, $6, 'human', 1, to_tsvector('simple', $7)) \
         ON CONFLICT (library_id, slug) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(lib)
    .bind(&slug)
    .bind(title)
    .bind(&content)
    .bind(sqlx::types::Json(&fm))
    .bind(page_tsv_text(&slug, title, &content))
    .execute(&self.pool)
    .await?;

        // 2) 再摄取（实体概念网络吸收本次问答内容）——D27 三态：仅已就绪算 skipped
        let outcome = crate::ingest::enqueue_ingest(&self.queue, lib, title, &content).await?;
        Ok((outcome.skipped(), slug))
    }

    /// write_page 织入钩子（2026-09-13）：AI 写页后自动把页面当原料入队再摄取
    /// （吸收概念/实体/互链，不级联重建全库）——「写入即处理」。
    /// 同内容（sha）去重内建：已 ready 跳过、在途 InFlight、failed 才重提。
    pub async fn auto_ingest_page(
        &self,
        lib: Uuid,
        title: &str,
        content: &str,
    ) -> Result<serde_json::Value, WikiError> {
        let outcome = crate::ingest::enqueue_ingest(&self.queue, lib, title, content).await?;
        let v = match outcome {
            crate::ingest::IngestOutcome::Enqueued(id, job) => serde_json::json!({
                "state": "enqueued", "source_id": id, "job_id": job,
                "hint": "已入队织入（analyze→generate，任务页可见）——概念吸收与互链稍后出现",
            }),
            crate::ingest::IngestOutcome::AlreadyReady(id) => serde_json::json!({
                "state": "already_ingested", "source_id": id,
                "hint": "同内容已织入过（sha 命中）——跳过",
            }),
            crate::ingest::IngestOutcome::InFlight(id, job) => serde_json::json!({
                "state": "in_flight", "source_id": id, "job_id": job,
                "hint": "织入在途（同内容正在处理）",
            }),
        };
        Ok(v)
    }

    /// 存量回填（D4 遗留）：重析全部页面正文重建 wiki_links（库内）。
    /// 修复前写入的页面链接索引缺失——一次性全量重析（幂等，先清后建）。
    pub async fn rebuild_all_links(&self, lib: Uuid) -> Result<u64, WikiError> {
        let pages: Vec<(String, String)> = sqlx::query_as(
            "SELECT slug, content FROM wiki_pages WHERE library_id = $1 ORDER BY slug",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        sqlx::query("DELETE FROM wiki_links WHERE library_id = $1")
            .bind(lib)
            .execute(&self.pool)
            .await?;
        let mut n = 0;
        for (slug, content) in &pages {
            for target in crate::markup::extract_wikilinks(content) {
                if crate::markup::split_cross_lib(&target).is_some() {
                    continue; // 跨库引用不进库内 wiki_links（graph/孤页检测是库内概念）
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
                .await?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Merge：新陈代谢的合并原语（wiki 收录哲学线工单④，AI 处置重复 flag 与 repair 共用）。
    /// duplicate 并入 primary：正文为空或为 primary 子串 → 冗余丢弃；否则整段并入「合并自」章节；
    /// primary 自身引用 dup 的链接去链接化（改指会变自链）；全库其他页指向 dup 的链接改指 primary；
    /// delete_page(dup)（版本快照兜底——下架不烧书）。返回人话明细。
    /// 规模化 task-4：概念页内容级合并——重复候选检测（聚合出口）。
    /// v1 检测口径：标题归一化（lower+trim）相同的页面组——LLM 从不同来源织入
    /// 产出同名页是最常见的重复形态。研判流：duplicate_candidates → AI/人工
    /// 逐组研判 → merge_pages 合并（留痕）或确认共存。
    pub async fn duplicate_candidates(
        &self,
        lib: Uuid,
    ) -> Result<Vec<serde_json::Value>, WikiError> {
        let rows: Vec<(String, i64, serde_json::Value)> = sqlx::query_as(
            "SELECT lower(btrim(title)) AS norm_title, count(*) AS cnt, \
            json_agg(json_build_object(\
                'slug', slug, 'title', title, 'page_type', page_type, \
                'folder', folder, 'version', version, 'updated_at', updated_at) \
                ORDER BY updated_at DESC) AS pages \
        FROM wiki_pages \
        WHERE library_id = $1 AND page_type NOT IN ('index','log','overview') \
          AND slug NOT LIKE 'community-synthesis-%' \
        GROUP BY lower(btrim(title)) HAVING count(*) > 1 \
        ORDER BY count(*) DESC, norm_title",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WikiError::Storage(e.to_string()))?;
        Ok(rows
        .into_iter()
        .map(|(norm_title, cnt, pages)| {
            serde_json::json!({ "norm_title": norm_title, "count": cnt, "pages": pages })
        })
        .collect())
    }
}
