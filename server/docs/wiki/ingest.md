# 两步 Ingest

Wiki 摄取是核心链路：一篇原料（文本或 knowledge 文档）经「入队 → 分析 → 生成」三步，变成互链的 wiki 页面。所有实现位于 `crates/wiki-engine/src/ingest.rs`。

## 总览

```text
enqueue_ingest ──► wiki_analyze job ──► wiki_generate job
  (幂等落原料)       (LLM 分析,中档)       (LLM 生成,高档)
```

- `enqueue_ingest`：同步入口（`WikiService::ingest` 调用），做幂等判断 + 原料落盘 + 入队第一个 job。
- `wiki_analyze`：异步 job，LLM 读原料 + 既有 index + purpose，产出结构化分析。
- `wiki_generate`：异步 job，LLM 读分析 + 原料 + 既有页面，产出/更新页面。

两个 job 通过 payload 串联：analyze 成功后 `ctx.enqueue_next` 显式入队 generate，`analysis` 结果整体塞进 generate 的 payload。

## 1. 入队：enqueue_ingest

签名：`enqueue_ingest(queue, title, text) -> (source_id, skipped)`。

步骤：

1. **sha256 幂等**：`sha256(text)` → 查 `wiki_sources` 是否已有同 sha 且 `status='ready'`。命中直接返回 `(id, skipped=true)`，整个 ingest 秒跳过。
2. **原料落盘**：写 `AGENT_MEMORY_DATA_DIR/wiki-sources/{id}.md`（id 为 `Uuid::now_v7()`）。原料不可变，此后 LLM 只读这份副本，绝不回改原文。
3. **写 wiki_sources**：`INSERT ... status='pending'`，`ON CONFLICT (sha256) DO UPDATE SET title, status='pending' RETURNING id`。

   > ⚠️ 注意 `RETURNING id`：sha 冲突时返回的是**旧行 id**，不是新生成的 uuid。后续 payload 必须用这个 `real_id`，否则 analyze 读不到原料行会 no-rows 死循环（Phase 7 审计修复的历史 bug，见代码注释）。
4. **冲突内容覆盖**：若 `real_id != id`，删掉新落的临时文件，改写到 `{real_id}.md` 并回写 raw_path。
5. **入队**：`wiki_analyze` job，payload `{"source_id": real_id}`，幂等键 `wiki-analyze-{real_id}`。

> knowledge 文档入口：`WikiService::ingest_knowledge_document` 先 `parsing::parse_bytes` 解出纯文本，再复用 `enqueue_ingest`。

## 2. 第一步：wiki_analyze（分析）

job handler：`analyze_job(ctx, llm)`。

步骤：

1. `wiki_sources.status → processing`。
2. 读原料全文 + 既有 index（`read_index`：最多 200 个页面的 `[[slug]] 标题` 列表）+ purpose 上下文。
3. **LLM 调用**：`Purpose::WikiAnalysis`（中档），system 为 `prompts::analysis_system()`，user 拼装为：

   ```text
   == 知识库 Purpose == {purpose_markdown}
   == 现有页面目录 == {index}
   == 源文档 == {text}
   ```

4. 输出**严格 JSON**（schema 见 `prompts.rs`，`P_WIKI_ANALYSIS` v1）：

   ```json
   {
     "entities":  ["实体名"],
     "concepts":  ["概念名"],
     "links":     [{"slug": "既有页", "reason": "为何相关"}],
     "conflicts": [{"slug": "既有页", "issue": "矛盾点"}],
     "source_title": "建议的源摘要页标题",
     "reviews":   [{"kind": "create_page|deep_research|skip|flag",
                    "title": "...", "reason": "...",
                    "suggested_slug": "可空", "search_queries": ["检索词"]}],
     "purpose_suggestion": {"goals": [...], "key_questions": [...], "reason": "..."} | null
   }
   ```

5. **review flag 落库**：把 `reviews[]` 解析成 `LlmReviewFlag`，经 `review::create_items` 写入 `wiki_review_items`（非法 kind 丢弃）。**不阻塞 ingest**——人审异步进行。
6. **purpose 建议入人审队列**：若 `purpose_suggestion` 是对象，插入一条 `wiki_review_items`（kind=`flag`，payload 为建议）。LLM 只建议不直接改 purpose。
7. **链式入队**：`wiki_generate` job，payload 含 `source_id` + 完整 `analysis` + `source_title`，幂等键 `wiki-generate-{source_id}`。

## 3. 第二步：wiki_generate（生成）

job handler：`generate_job(ctx, llm)`。

步骤：

1. 读原料 + 既有页面集合 + purpose + 分析结果。
2. **LLM 调用**：`Purpose::WikiGeneration`（高档），`prompts::generation_system()`。规则要点：
   - 为每个 entity/concept 生成一页；已在既有集合里的**不重建**，把更新并入该页（version+1 的完整新内容）。
   - 页面格式：第一行 `# 标题`，正文 3~8 句中文，相关处 `[[页面名]]` 互链。
   - 生成 source 摘要页（page_type=source）。
   - 有多源相关实体时生成 synthesis 综合页；有 conflicts 时生成 comparison 对比页。
3. 输出严格 JSON：`{"pages": [{"slug", "page_type", "title", "content"}]}`。
4. **逐页落地**（对每个 page，跳过非法 slug 或空 content）：

   | 情形 | 处理 |
   |---|---|
   | slug 不存在 | `INSERT` 新页，`origin='llm'`，version=1，`created += 1` |
   | 已存在且 `origin='human'` | **不覆盖**，emit「人工页面更新提案」事件等 UI 确认合入，`proposals += 1` |
   | 已存在且 `origin='llm'` | `UPDATE` 内容，`frontmatter.sources` 合并去重（追加当前 source_id），version+1，`updated += 1` |

5. **重建链接**：`rebuild_links`——对本批页面重新提取 `[[wikilink]]`，清掉旧出边再插入 `wiki_links`（weight 3.0）。
6. **维护系统页**：`update_index_and_log`——重建 `index` 页；`log` 页追加一行操作记录（全量历史靠 job_events）。
7. **嵌入 + 检索字段**：`collect_page_texts`（`标题\n正文`）→ `llm.embed`（1024 维）→ `write_embeddings` 回写 `embedding` + `tsv`。
8. `wiki_sources.status → ready`，`last_ingested_at = now()`。
9. **若 created/updated > 0**：重建 `overview` 页 + `relevance::rebuild_weights` 全量重算链接权重（详见 [graph.md](graph.md)），emit 汇总事件。

## source 状态机

```text
pending ──► processing ──► ready
              │
              └──(job 失败重试)──► 回到 pending 重跑
```

CHECK 约束含 `failed`，但当前代码未显式置 `failed`（失败靠 jobs 队列的重试/死信机制兜底）。死任务可在 UI 用 `WikiService::reingest` 重新入队 `wiki_analyze`。

## LLM 调用可靠性

- 统一走 `distill::llm_port::chat_json_retrying`：JSON 解析失败自动追加「只输出合法 JSON」指令**重试一次**；两次都失败则 job 失败。
- 每次调用（含重试）完整 I/O 记入 `job_events`，含 purpose 标签，可归因可回放。
- `MockLlm` 供测试注入（FIFO 响应队列 + 确定性伪向量），`GatewayLlm` 走真实 `ProviderRegistry`（路由 + 用量记账 + token 预算熔断）。

## 关键文件

- `crates/wiki-engine/src/ingest.rs` — 全部 ingest 逻辑 + `register_handlers`
- `crates/wiki-engine/src/prompts.rs` — `analysis_system` / `generation_system`（版本化）
- `crates/wiki-engine/src/purpose.rs` — purpose 上下文注入
- `crates/wiki-engine/src/review.rs` — review flag 落库
- `crates/distill/src/llm_port.rs` — `DistillLlm` / `chat_json_retrying`
