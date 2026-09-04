# Wiki 概览

## 是什么

Wiki 是 Engram 平台的四类长期记忆资产之一，实现 **Karpathy LLM-wiki** 模式：

- **原料不可变**：每一篇喂进来的源文档（source）先落一份只读副本（`AGENT_MEMORY_DATA_DIR/wiki-sources/{id}.md`），内容永不回改。
- **LLM 增量维护**：LLM 读原料 + 既有页面目录，增量产出/更新互链的 wiki 页面，而不是每次全量重写。
- **人负责纠偏**：LLM 的输出不是终态——review 队列、purpose 方向、人工编辑保护（origin=human）三条线把「最终话语权」留给人。

一句话：平台不替你做知识库，而是给 LLM 一个「按你的意图、可被你把关」的持续写作工具。

## 核心概念

| 概念 | 说明 |
|---|---|
| **source（原料）** | 一篇不可变输入（文本或 wiki 文档），落盘 + 记 sha256 |
| **page（页面）** | 由 LLM 生成/维护的 Markdown 页，有 slug、page_type、origin、version |
| **wikilink** | `[[slug]]` / `[[slug\|显示名]]` 互链语法，是链接图的边来源 |
| **frontmatter** | 每页的 jsonb 元数据，含 `sources[]`（该页由哪些原料蒸馏而来） |
| **origin** | `llm`（机器写的）或 `human`（人写的）；human 页 LLM 只提案不覆盖 |
| **purpose** | wiki 的「方向意图」，ingest/query 时注入 LLM，可被 LLM 建议更新 |
| **review** | LLM 在 ingest 时 flag 出的待人审项（建页/深研/跳过/其他） |

## 页面类型（page_type）

10 种枚举（见 [data-model.md](data-model.md)）：

| 类型 | 谁产出 | 用途 |
|---|---|---|
| `entity` | LLM | 实体页（人物/组织/产品/技术） |
| `concept` | LLM / 人工 | 概念页（理论/方法/主题）；人工 `PUT /wiki/pages/{slug}` 默认落此型 |
| `source` | LLM | 原料摘要页（文档 2~4 句摘要 + 要点） |
| `synthesis` | LLM | 跨源综合页（多源观点共性/分歧） |
| `comparison` | LLM | 对比页（conflicts 触发，逐维度对比） |
| `queries` | 人工 | 检索问答存档页（`archive_query`） |
| `index` | 系统 | 知识库索引页（ingest 后重建） |
| `log` | 系统 | 操作日志页（ingest 后追加） |
| `overview` | 系统 | 全局总览页（每页一行摘要） |
| `purpose` | 保留 | migration CHECK 里有，但实际 purpose 存 settings 表，不落页 |

## 整体数据流

```text
文本 / wiki 文档
        │
        ▼
enqueue_ingest ──(sha256 幂等)──► wiki_sources(pending) + 原料落盘
        │
        ▼
   wiki_analyze job ──LLM WikiAnalysis(中档)──► 结构化分析
        │                                       ├─ review flag → wiki_review_items
        │                                       └─ purpose_suggestion → 人审队列
        ▼
   wiki_generate job ──LLM WikiGeneration(高档)──► 页面新建/更新/提案
        │
        ├─► wiki_links（wikilink 边，weight 3.0）
        ├─► index / log / overview 系统页维护
        ├─► 嵌入（1024 维）+ tsv 更新
        ├─► 4 信号相关性权重重算（source-overlap 补边 weight 4.0）
        └─► wiki_sources → ready
```

数据流之外，还有三条「读 + 纠偏」线：

- **读**：`/wiki/pages` 浏览、`/wiki/graph` 链接图、`/wiki/search` 检索（FTS+向量）、`/wiki/insights` 图洞察
- **纠偏**：`/wiki/lint` 健康检查、`/wiki/reviews` 人审、`/wiki/purpose` 方向调整、`/wiki/sources/{id}` 级联删除

## 关键设计特征

- **两步 ingest**：analysis（读懂 + 找关联/冲突/人审项）与 generation（写页）分离，中间产物可人审，避免「黑盒直写」。
- **幂等**：source 按 sha256 去重，同内容重复 ingest 秒跳过；job 用 idempotency_key（`wiki-analyze-{id}` / `wiki-generate-{id}`）防重复执行。
- **origin 保护**：人写的页面 LLM 永不覆盖，只生成「提案事件」等人确认合入。
- **LLM 唯一出口**：分析走 `Purpose::WikiAnalysis`（中档），生成走 `Purpose::WikiGeneration`（高档），统一走 `llm` crate 的路由/记账/加密。
- **检索同源**：页面正文写 tsv（`simple` 配置），嵌入写 pgvector HNSW，检索时 FTS + 向量 RRF 融合。
- **可归因可回放**：每步 LLM 调用的完整 I/O 记入 job_events，含 `prompt_version`（见 ingest.rs 的 `chat_json_retrying`）。

详见 [ingest.md](ingest.md)（链路）、[graph.md](graph.md)（图）、[operations.md](operations.md)（纠偏）。

## 延伸阅读

- [theory.md](theory.md) — 设计理论与来源：Karpathy 三层架构 / 三操作 / index+log，及理论到实现的映射
- [gap-analysis.md](gap-analysis.md) — 对照理论最佳实践与评论区实战教训的差距清单
