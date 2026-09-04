# 操作与纠偏

Wiki 不是「喂进去就完事」——它把最终话语权留给人。这里覆盖 5 条人工操作线：lint（体检）、review（人审）、purpose（方向）、级联删除（清理）、检索（消费）。

## lint（健康检查）

`POST /wiki/lint`，实现 `lint.rs::lint`，**只报告不改写**。产出 `LintReport { issues[], checked_pages }`，每条 issue 含 `rule / slug / detail`。

| 规则 | 判定 |
|---|---|
| `dead_link` | 页内 `[[wikilink]]` 指向不存在的页面 |
| `orphan` | 无任何入链的孤立页（系统页 index/log/overview 豁免） |
| `broken_frontmatter` | `frontmatter.sources` 缺失或非数组（系统页豁免） |
| `duplicate_entity` | 同标题但不同 slug 的 entity/concept 页 |
| `stale_source` | 原料已 ingest（last_ingested_at 非空）但无页面引用其内容 |

## review 系统

ingest 时 LLM 在分析阶段 flag 出待人审项（`analyze` 的 `reviews[]` 字段），异步落 `wiki_review_items`，**不阻塞 ingest**。

预定义 4 种动作（`valid_kind` 校验，防 LLM 幻觉任意动作）：

| kind | 含义 |
|---|---|
| `create_page` | 值得为它建独立页 |
| `deep_research` | 知识缺口，需检索补充 |
| `skip` | 内容存疑，建议跳过 |
| `flag` | 其他需人判断（含 purpose_suggestion 建议） |

操作：

- `GET /wiki/reviews` — 列出 open 项（最多 200，按 created_at 倒序）
- `POST /wiki/reviews/{id}/resolve` — 处理，body `{action?, dismiss}`；dismiss=true → `dismissed`，否则 `resolved`（可选记 action 标签）

非法 kind 在落库时被 `tracing::warn` 丢弃。

## purpose（wiki 灵魂）

方向意图，存在 `settings` 表的 `wiki_purpose` 键。四字段：`goals`（为什么建）、`key_questions`（应能回答什么）、`scope`（边界）、`thesis`（核心论点）。

- 注入点：ingest 的 analyze/generate 两个 LLM 调用，以及 `search_with_purpose` 检索（AI 客户端读 `query_context.purpose`）。
- 渲染：`Purpose::to_markdown()` 生成 `# Purpose` 形式的 Markdown 片段。
- LLM 可建议更新：analyze 输出 `purpose_suggestion` 时，插入 review 队列（kind=`flag`），**经人审才生效**，LLM 不直接改。

操作：`GET /wiki/purpose`、`PUT /wiki/purpose`（body `{goals, key_questions, scope, thesis}`）。

## 级联删除（cascade.rs）

`DELETE /wiki/sources/{id}`，删一个原料的全部下游。对齐 llm_wiki 的 3-method matching：

| 匹配路径 | 处理 |
|---|---|
| ① `frontmatter.sources[]` 含该 source id | 主路径，找到所有引用页 |
| ② `page_type='source'` 且唯一来源 | **整页删除**（摘要页随原料走） |
| ③ 共享 entity/concept 多源页 | **仅从 sources[] 移除该 source**，保留页面 |

后续清理：

1. **dead wikilink 清理**：剩余页面里指向已删 slug 的 `[[link]]` 移除（含多余空行合并）。
2. **index 同步**：`rebuild_index_page` 重建索引页。
3. **删 source 行**：`DELETE FROM wiki_sources`。

产出 `CascadeReport { deleted_pages[], updated_shared[], cleaned_links }`。

`GET /wiki/sources` 列出可删的 sources（id/title/status）。

## 检索（消费侧）

`POST /wiki/search`，body `{query, max_items?}`，默认 20、上限 50：

- `WikiService::search`：tsvector FTS（`to_tsquery('simple')` + `ts_rank` 排序）。
- 页面另有 `embedding`（pgvector HNSW 索引），供后续向量/混合检索复用。

`search_with_purpose` 额外返回 purpose 上下文包：`{purpose, pages}`——AI 客户端把 purpose 作为 system context 前缀，让检索回答对齐 wiki 方向。

## 其他人工操作

- `PUT /wiki/pages/{slug}` — 人工编辑页面（origin=human，version+1，重算 tsv）
- `POST /wiki/proposals/apply` — 人审通过后合入 LLM 提案（复用 `put_page`，保持 human 语义）
- `POST /wiki/queries/archive` — 问答/检索结果存档为 queries 页，并自动再 ingest 吸收实体概念
- `POST /wiki/ingest` — 手动触发 ingest（text 或 wiki 文档 document_id）
