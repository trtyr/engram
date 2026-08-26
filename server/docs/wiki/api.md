# Wiki HTTP API

实现：`crates/api/src/routes/wiki_api.rs`。所有端点需 `wiki` scope（`require_scope(p, "wiki")`）。

## 鉴权与错误

- 鉴权：Bearer token，Principal 需含 `wiki` scope，否则 403。
- 错误体统一 `{"error": {code, message, retryable}}`（`ApiError`）。
- `WikiError` → `ApiError` 映射：

| WikiError | HTTP |
|---|---|
| `NotFound` | 404 |
| `BadRequest` | 400 |
| `Storage` | 503 |

## 端点总览

| 方法 | 路径 | 说明 | 返回 |
|---|---|---|---|
| POST | `/wiki/ingest` | 触发两步 ingest | 202 `{skipped}` |
| GET | `/wiki/pages` | 列表页 | 200 `[WikiPageDto]` |
| GET | `/wiki/pages/{slug}` | 单页 | 200 `WikiPageDto` |
| PUT | `/wiki/pages/{slug}` | 人工编辑 | 200 `WikiPageDto` |
| GET | `/wiki/graph` | 链接图 | 200 `GraphDto` |
| POST | `/wiki/lint` | 健康检查 | 200 `LintReport` |
| POST | `/wiki/search` | 检索 | 200 `Object` |
| POST | `/wiki/proposals/apply` | 合入提案 | 200 `WikiPageDto` |
| GET | `/wiki/purpose` | 读方向意图 | 200 `Purpose`（可 null） |
| PUT | `/wiki/purpose` | 设方向意图 | 204 |
| GET | `/wiki/reviews` | 列出人审项 | 200 `[ReviewItem]` |
| POST | `/wiki/reviews/{id}/resolve` | 处理人审项 | 204 |
| POST | `/wiki/queries/archive` | 问答存档 | 202 `{skipped}` |
| GET | `/wiki/sources` | 列出原料 | 200 `[WikiSourceDto]` |
| DELETE | `/wiki/sources/{id}` | 级联删除 | 200 `CascadeReport` |
| POST | `/wiki/insights` | 图洞察 | 200 `InsightsReport` |
| POST | `/wiki/insights/dismiss` | dismiss 洞察 | 204 |
| POST | `/wiki/insights/reset` | 重置 dismiss | 204 |

## 核心 DTO

`WikiPageDto`：

```json
{"id", "slug", "title", "page_type", "content",
 "frontmatter", "origin", "version", "updated_at"}
```

## 端点详解

### POST /wiki/ingest

请求 `{title, text?, document_id?}`——`text` 与 `document_id` 二选一：

- `text`：直接文本
- `document_id`：knowledge 文档 id，服务端读 `documents.raw_path` + 重新 `parsing::parse_bytes` 解析

响应 202 `{skipped}`（sha 命中幂等跳过）。

### GET /wiki/pages

Query `{page_type?, limit?}`（limit 默认 100，上限 300）。排除 `log` 系统页，按 `updated_at` 倒序。

### PUT /wiki/pages/{slug}

请求 `{title, content}`。slug 非法 → 400。upsert：不存在则建 concept 页（origin=human），存在则 version+1 + origin=human + 重算 tsv。

### POST /wiki/search

请求 `{query, max_items?}`（默认 20，上限 50）。FTS 检索，返回 `{purpose, pages}`。

### POST /wiki/proposals/apply

请求 `{slug, title, content}`。复用 `put_page`（人审通过的内容保持 human 语义）。

### GET/PUT /wiki/purpose

PUT 请求 `{goals, key_questions?, scope?, thesis?}`，写 `settings[wiki_purpose]`。

### GET /wiki/reviews

返回 open 状态的 `ReviewItem[]`（最多 200）。

### POST /wiki/reviews/{id}/resolve

请求 `{action?, dismiss}`。dismiss=true → `dismissed`，否则 `resolved`（可带 action 标签）。id 不存在或已处理 → 错误。

### POST /wiki/queries/archive

请求 `{title, question, answer}`。落 queries 页（origin=human）并自动再 ingest。响应 202 `{skipped}`。

### GET /wiki/sources

返回 `[{id, title, status}]`，供 UI 列出可删原料。

### DELETE /wiki/sources/{id}

级联删除（见 [operations.md](operations.md)），返回 `CascadeReport {deleted_pages, updated_shared, cleaned_links}`。

### POST /wiki/insights

图洞察，返回 `InsightsReport {insights, communities, total_pages}`（见 [graph.md](graph.md)）。

### POST /wiki/insights/dismiss

请求 `{key}`，写入 dismiss。204。

### POST /wiki/insights/reset

清空全部 dismiss。204。

## 系统页约定

- `index` / `log` / `overview` 是系统页，由 ingest 自动维护，不在普通列表里（`list_pages` 只排除 `log`，`read_index`/`rebuild_*` 排除 index/log/overview）。
- `log` 页只存最新操作记录（全量历史在 job_events）。
