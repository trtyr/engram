# Wiki 数据模型

schema 唯一定义在 `server/migrations/0007_wiki.sql`（核心三表）与 `0012_wiki_align.sql`（review + 洞察 dismiss）。purpose 复用 `0010_settings.sql` 的 settings 表。

## 表一览

| 表 | 迁移 | 用途 |
|---|---|---|
| `wiki_sources` | 0007 | 原料（source），不可变输入 |
| `wiki_pages` | 0007 | wiki 页面 |
| `wiki_links` | 0007 | 页面链接图（有向边 + 权重） |
| `wiki_review_items` | 0012 | LLM flag 的人审项 |
| `wiki_insight_dismissals` | 0012 | 图洞察的 dismiss 记录 |
| `settings` | 0010 | `wiki_purpose` 键（wiki 方向意图） |

## wiki_sources（原料）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | uuid PK | `Uuid::now_v7()` |
| `sha256` | text UNIQUE | 内容哈希，幂等键 |
| `raw_path` | text | 原料副本路径（`{data_dir}/wiki-sources/{id}.md`） |
| `title` | text | 可选标题 |
| `status` | text CHECK | `pending` / `processing` / `ready` / `failed` |
| `last_ingested_at` | timestamptz | 最近成功 ingest 时间 |
| `created_at` | timestamptz | 创建时间 |

状态流转：`pending → processing → ready`。代码当前不显式置 `failed`（失败靠 jobs 队列重试兜底）。

## wiki_pages（页面）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | uuid PK | |
| `slug` | text UNIQUE | 页面标识，规则见下 |
| `title` | text NOT NULL | 标题 |
| `page_type` | text CHECK | 10 种枚举（见下） |
| `content` | text NOT NULL | Markdown 正文 |
| `frontmatter` | jsonb | 元数据，含 `sources[]` |
| `origin` | text CHECK | `llm` / `human` |
| `version` | int | 递增版本，默认 1 |
| `folder` | text | 目录树层级（`/` 分隔多级，Obsidian 式；蒸馏按 page_type 归文件夹，人工可改） |
| `embedding` | vector(1024) | 页面嵌入（标题+正文） |
| `tsv` | tsvector | 全文检索字段（`simple` 配置） |
| `created_at` / `updated_at` | timestamptz | |

索引：

```sql
CREATE INDEX idx_wiki_pages_type      ON wiki_pages (page_type);
CREATE INDEX idx_wiki_pages_tsv       ON wiki_pages USING gin (tsv);
CREATE INDEX idx_wiki_pages_embedding ON wiki_pages USING hnsw (embedding vector_cosine_ops);
```

### page_type 枚举

| 值 | 产出者 | 语义 |
|---|---|---|
| `entity` | LLM | 实体页 |
| `concept` | LLM / 人工 | 概念页；人工 `put_page` 默认此型 |
| `source` | LLM | 原料摘要页 |
| `synthesis` | LLM | 跨源综合页 |
| `comparison` | LLM | 对比页 |
| `queries` | 人工 | 问答存档页 |
| `index` | 系统 | 索引页 |
| `log` | 系统 | 操作日志页 |
| `overview` | 系统 | 总览页 |
| `purpose` | 保留 | 实际 purpose 存 settings，不落页 |

### origin 语义（人工编辑保护核心）

| 值 | 含义 | 对 LLM 的影响 |
|---|---|---|
| `llm` | 机器写的 | 后续 ingest 可直接更新（version+1，合并 sources） |
| `human` | 人写的 | **LLM 永不覆盖**，只 emit「提案事件」等 UI 确认合入 |

`PUT /wiki/pages/{slug}`（人工编辑）会把 origin 置为 `human` 并 version+1。

### slug 规则（markup.rs `is_valid_slug`）

- 非空，≤ 80 字符
- 禁 `/`、`\`、空白
- 仅允许字母数字 + `-` `_` `·`

### frontmatter 结构

```json
{
  "title": "页面标题",
  "page_type": "concept",
  "sources": ["<source_id>", "..."]
}
```

`sources[]` 是核心溯源字段：记录该页由哪些原料蒸馏而来，级联删除与 4 信号相关性都依赖它。

## wiki_links（链接图）

| 字段 | 类型 | 说明 |
|---|---|---|
| `from_slug` | text PK | 出边页 |
| `to_slug` | text PK | 入边页 |
| `weight` | real | 边权重，默认 3.0 |

两种边：**wikilink 边**（初始 3.0）与 **source-overlap 边**（4.0，见 [graph.md](graph.md)）。复合主键 `(from_slug, to_slug)`，索引 `idx_wiki_links_to`（按 to_slug 查入链，供 lint 用）。

## wiki_review_items（人审项）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | uuid PK | |
| `kind` | text CHECK | `create_page` / `deep_research` / `skip` / `flag` |
| `payload` | jsonb | `{title, reason, suggested_slug}` |
| `action` | text | 处理时选择的动作标签 |
| `search_queries` | jsonb | LLM 预生成的检索词 |
| `source_id` | uuid FK | 关联原料，`ON DELETE SET NULL` |
| `status` | text CHECK | `open` / `resolved` / `dismissed` |
| `created_at` / `resolved_at` | timestamptz | |

部分索引 `idx_wiki_review_open`（status='open'）加速列出待办。

## wiki_insight_dismissals（洞察 dismiss）

| 字段 | 类型 | 说明 |
|---|---|---|
| `insight_key` | text PK | 稳定键，格式 `类型:slug`（如 `isolated_page:foo`） |
| `created_at` | timestamptz | |

## purpose（settings 表）

键 `wiki_purpose`，值 jsonb：

```json
{
  "goals":          ["为什么建这个知识库"],
  "key_questions":  ["wiki 应能回答什么"],
  "scope":          ["研究范围边界"],
  "thesis":         "演化中的核心论点（可空）"
}
```

由 `purpose.rs` 的 `Purpose` struct 读写，ingest/query 时渲染成 Markdown 注入 LLM。
