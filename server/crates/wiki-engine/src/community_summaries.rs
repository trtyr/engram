//! 社区摘要层（批次③ GraphRAG 式，wiki 大库化 2026-09-19）：
//! Louvain 社区 → synthesis 综述页参与召回（RAPTOR/llm_wiki 社区摘要思想）。
//!
//! 设计：
//! - 触发：织入流水线尾部（overview 重生成之后）；失败只告警不阻塞织入
//! - slug 稳定：`community-synthesis-{hash16}`（hash 由成员 slug 集合派生——成员不变
//!   slug 不变，检索与双链引用不因社区重编号漂移）
//! - 增量：frontmatter.community_hash 与重算结果对比，成员不变的社区不重调 LLM（成本守卫）
//! - 清理：消失社区的摘要页删除（连带清双向 wiki_links；摘要页可再生，不留版本快照）
//! - 图排除：摘要页自身不进 Louvain 图（防「摘要页自成社区 → 自己综述自己」滚雪球）

use std::collections::HashMap;

use engram_jobs::JobContext;
use engram_jobs::types::JobError;
use serde_json::json;
use sha2::Digest;
use uuid::Uuid;

const SYNTHESIS_SLUG_PREFIX: &str = "community-synthesis-";
/// 社区最少成员数——3 页以下的「社区」没有综述价值。
const MIN_COMMUNITY_SIZE: usize = 3;
/// 图太小（非系统页 < 6）不划社区。
const MIN_GRAPH_NODES: usize = 6;
/// 成员首段摘要截断长度（字符）。
const MEMBER_SNIPPET_CHARS: usize = 200;

/// 纯逻辑：成员 slug 集合 → 稳定社区 hash（排序后 sha256 前 16 位）。
pub(crate) fn community_hash(members: &[String]) -> String {
    let mut sorted: Vec<&String> = members.iter().collect();
    sorted.sort();
    let joined = sorted
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let mut h = sha2::Sha256::new();
    h.update(joined.as_bytes());
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    hex[..16].to_string()
}

/// 纯逻辑：提取首段摘要（跳过 `#` 标题行与空行，取第一个非空段截断）。
pub(crate) fn first_snippet(content: &str) -> String {
    let para = content
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let chars: Vec<char> = para.chars().collect();
    if chars.len() <= MEMBER_SNIPPET_CHARS {
        para.to_string()
    } else {
        let s: String = chars[..MEMBER_SNIPPET_CHARS].iter().collect();
        format!("{s}…")
    }
}

/// 纯逻辑：louvain 分配结果 → 社区分组（过滤 < MIN_COMMUNITY_SIZE，按社区 id 排序）。
pub(crate) fn group_communities<'a>(
    assignment: &HashMap<&'a str, usize>,
) -> Vec<(usize, Vec<&'a str>)> {
    let mut by: HashMap<usize, Vec<&'a str>> = HashMap::new();
    for (slug, &c) in assignment {
        by.entry(c).or_default().push(slug);
    }
    let mut out: Vec<(usize, Vec<&str>)> = by
        .into_iter()
        .filter(|(_, m)| m.len() >= MIN_COMMUNITY_SIZE)
        .collect();
    out.sort_by_key(|(c, _)| *c);
    out
}

/// 主入口：织入尾部调用。返回统计 {created, deleted, skipped}。
pub async fn refresh_community_summaries(
    pool: &sqlx::PgPool,
    llm: &crate::service::LlmRef,
    lib: Uuid,
    ctx: &JobContext,
) -> Result<serde_json::Value, JobError> {
    // 1. 库内非系统页（排除摘要页自身——不进图）
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages \
         WHERE library_id = $1 AND page_type NOT IN ('index','log','overview') \
         AND slug NOT LIKE $2",
    )
    .bind(lib)
    .bind(format!("{SYNTHESIS_SLUG_PREFIX}%"))
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    if rows.len() < MIN_GRAPH_NODES {
        return Ok(json!({"created": 0, "deleted": 0, "skipped": 0, "reason": "graph_too_small"}));
    }

    // 2. 库内双链边（weight 列 FLOAT4——SQL 层 ::float8，防 sqlx 解码类型错配）
    let edges: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT from_slug, to_slug, weight::float8 FROM wiki_links WHERE library_id = $1",
    )
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 3. Louvain 划社区。万页保护（2026-09-20 压测发现）：纯 CPU 计算会阻塞 tokio worker——
    // 后台 job 不做规模降级（摘要层是维护能力，大库同样要摘要），但 spawn_blocking 隔离。
    // 闭包按索引回传（不携借用），外层再与 nodes 重建 slug→社区映射。
    let nodes: Vec<String> = rows.iter().map(|(s, _, _)| s.clone()).collect();
    let assign_ids: Vec<usize> = {
        let nodes_c = nodes.clone();
        let edges_c = edges.clone();
        tokio::task::spawn_blocking(move || {
            let m = crate::community::louvain_communities(&nodes_c, &edges_c);
            nodes_c
                .iter()
                .map(|n| m.get(n.as_str()).copied().unwrap_or(0))
                .collect()
        })
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
    };
    let assignment: HashMap<&str, usize> =
        nodes.iter().map(|n| n.as_str()).zip(assign_ids).collect();
    let groups = group_communities(&assignment);

    // 4. 已有摘要页：hash → slug
    let existing: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT slug, frontmatter->>'community_hash' FROM wiki_pages \
         WHERE library_id = $1 AND slug LIKE $2",
    )
    .bind(lib)
    .bind(format!("{SYNTHESIS_SLUG_PREFIX}%"))
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let existing: HashMap<String, String> = existing
        .into_iter()
        .filter_map(|(slug, h)| h.map(|h| (h, slug)))
        .collect();

    let title_by_slug: HashMap<&str, &str> = rows
        .iter()
        .map(|(s, t, _)| (s.as_str(), t.as_str()))
        .collect();
    let content_by_slug: HashMap<&str, &str> = rows
        .iter()
        .map(|(s, _, c)| (s.as_str(), c.as_str()))
        .collect();

    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut deleted = 0usize;
    let mut new_pages: Vec<(String, String, String)> = Vec::new(); // slug, title, content
    let mut new_hashes: Vec<String> = Vec::new();

    // 5. 逐社区：hash 命中跳过；否则 LLM 生成综述页
    for (_, members) in &groups {
        let member_slugs: Vec<String> = members.iter().map(|s| s.to_string()).collect();
        let hash = community_hash(&member_slugs);
        new_hashes.push(hash.clone());
        if existing.contains_key(&hash) {
            skipped += 1;
            continue;
        }
        let member_lines: Vec<String> = members
            .iter()
            .map(|s| {
                format!(
                    "- {} | {} | {}",
                    s,
                    title_by_slug.get(*s).copied().unwrap_or(s),
                    first_snippet(content_by_slug.get(*s).copied().unwrap_or(""))
                )
            })
            .collect();
        let user = format!(
            "== 主题社区成员（{} 页）==\n{}\n\n请生成该社区的主题综述页。",
            members.len(),
            member_lines.join("\n")
        );
        // 单社区 LLM 失败不拖垮整层——留待下一轮织入重试
        let out = match engram_distill::llm_port::chat_json_retrying(
            ctx,
            llm.as_ref(),
            engram_llm::types::Purpose::WikiGeneration,
            &crate::prompts::community_synthesis_system(),
            &user,
            ctx.job.id,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "社区综述 LLM 生成失败（本社区跳过）");
                continue;
            }
        };
        let title = out
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "主题综述".into());
        let content = out
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if content.is_empty() {
            continue;
        }
        let slug = format!("{SYNTHESIS_SLUG_PREFIX}{hash}");
        let fm = json!({
            "title": title,
            "page_type": "synthesis",
            "community_hash": hash,
            "community_members": member_slugs,
            "origin_if_new": "llm",
        });
        sqlx::query(
            "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, folder) \
             VALUES ($1, $2, $3, $4, 'synthesis', $5, $6::jsonb, 'llm', 1, $7) \
             ON CONFLICT (library_id, slug) DO UPDATE SET \
                title = $4, content = $5, frontmatter = $6::jsonb, \
                version = wiki_pages.version + 1, updated_at = now()",
        )
        .bind(Uuid::now_v7())
        .bind(lib)
        .bind(&slug)
        .bind(&title)
        .bind(&content)
        .bind(sqlx::types::Json(&fm))
        .bind(crate::service::folder_for_type("synthesis"))
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        new_pages.push((slug, title, content));
        created += 1;
    }

    // 6. 清理消失社区的摘要页（连带清双向链接——可再生页，不留版本快照）
    let fresh: std::collections::HashSet<&String> = new_hashes.iter().collect();
    for (hash, slug) in &existing {
        if !fresh.contains(hash) {
            sqlx::query("DELETE FROM wiki_pages WHERE slug = $1 AND library_id = $2")
                .bind(slug)
                .bind(lib)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            sqlx::query(
                "DELETE FROM wiki_links WHERE (from_slug = $1 OR to_slug = $1) AND library_id = $2",
            )
            .bind(slug)
            .bind(lib)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            deleted += 1;
        }
    }

    // 7. 新页 tsv + embedding（嵌入失败不阻塞——FTS 已可召回）
    for (slug, title, content) in &new_pages {
        let text = engram_search::tokenize::tsv_text_wiki(&format!("{slug} {title} {content}"));
        sqlx::query("UPDATE wiki_pages SET tsv = to_tsvector('simple', $3) WHERE slug = $1 AND library_id = $2")
            .bind(slug)
            .bind(lib)
            .bind(&text)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    if !new_pages.is_empty() {
        let texts: Vec<String> = new_pages
            .iter()
            .map(|(_, t, c)| format!("{t}\n{c}"))
            .collect();
        match llm.embed(&texts, ctx.job.id).await {
            Ok(emb)
                if emb.len() == texts.len()
                    && emb.iter().all(|v| {
                        v.len() == engram_distill::llm_port::embedding_dimensions() as usize
                    }) =>
            {
                for (i, (slug, _, _)) in new_pages.iter().enumerate() {
                    sqlx::query(
                        "UPDATE wiki_pages SET embedding = $3 WHERE slug = $1 AND library_id = $2",
                    )
                    .bind(slug)
                    .bind(lib)
                    .bind(pgvector::Vector::from(emb[i].clone()))
                    .execute(pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                }
            }
            Ok(emb) => {
                tracing::warn!(
                    expected = texts.len(),
                    got = emb.len(),
                    "社区综述嵌入响应与批次不符，向量放弃（FTS 不受影响）"
                );
                let _ = emb;
            }
            Err(e) => {
                tracing::warn!(error = %e, "社区综述嵌入失败（FTS 不受影响）");
            }
        }
    }

    Ok(json!({"created": created, "deleted": deleted, "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn hash_is_stable_and_order_insensitive() {
        let a = community_hash(&s(&["x", "a", "m"]));
        let b = community_hash(&s(&["m", "x", "a"]));
        let c = community_hash(&s(&["x", "a"]));
        assert_eq!(a, b, "同集合不同顺序 → 同 hash");
        assert_ne!(a, c, "不同集合 → 不同 hash");
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn snippet_skips_title_and_truncates() {
        let md = "# 标题\n\n第一段正文内容。\n\n第二段。";
        assert_eq!(first_snippet(md), "第一段正文内容。");
        let long: String = "字".repeat(500);
        let snip = first_snippet(&long);
        assert_eq!(snip.chars().count(), MEMBER_SNIPPET_CHARS + 1); // 200 + '…'
        assert!(snip.ends_with('…'));
    }

    #[test]
    fn grouping_filters_small_communities() {
        let nodes = s(&["a", "b", "c", "d", "e", "f", "g"]);
        let edges: Vec<(String, String, f64)> = [
            ("a", "b", 3.0),
            ("b", "c", 3.0),
            ("a", "c", 3.0),
            ("d", "e", 3.0),
            ("e", "f", 3.0),
            ("d", "f", 3.0),
        ]
        .iter()
        .map(|(a, b, w)| (a.to_string(), b.to_string(), *w))
        .collect();
        let assignment = crate::community::louvain_communities(&nodes, &edges);
        let groups = group_communities(&assignment);
        // {a,b,c} 与 {d,e,f} 成组；g 落单（<3）被过滤
        assert_eq!(groups.len(), 2);
        for (_, m) in &groups {
            assert_eq!(m.len(), 3);
        }
    }
}
