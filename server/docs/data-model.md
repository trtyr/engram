# 数据模型

> 来源：`server/migrations/` 12 个迁移文件（schema 唯一定义处，启动时由 sqlx 自动执行）。
> 约定：所有 ID 用 `uuid`（v7），时间戳 `timestamptz`，embedding 统一 **1024 维**（bge-m3），
> 全文检索的 `tsv`（tsvector）由应用层 jieba 预分词维护（写入与查询同源）。

## 扩展

| 迁移 | 扩展                 | 用途                                      |
| ---- | -------------------- | ----------------------------------------- |
| 0001 | `vector`（pgvector） | 向量存储 + HNSW 索引                      |
| 0009 | `pg_trgm`            | 子串/模糊匹配（chunks、atoms 的 content） |

## 表总览

| 域        | 表                                                         | 迁移                    |
| --------- | ---------------------------------------------------------- | ----------------------- |
| 系统      | `jobs` / `job_events`                                      | 0002                    |
| 系统      | `llm_providers` / `llm_usage`                              | 0003                    |
| 系统      | `api_keys` / `admin_sessions`                              | 0004                    |
| 系统      | `settings`                                                 | 0010                    |
| 记忆      | `raw_sessions` / `atoms` / `scenarios` / `persona_aspects` | 0005（0011 扩充 atoms） |
| 知识      | `documents` / `chunks`                                     | 0006                    |
| Wiki      | `wiki_sources` / `wiki_pages` / `wiki_links`               | 0007                    |
| Wiki      | `wiki_review_items` / `wiki_insight_dismissals`            | 0012                    |
| CodeGraph | `cg_projects`                                              | 0008                    |

## 系统域

### jobs — 任务队列

所有长操作（蒸馏/摄取/ingest/同步）的执行单元。状态机：`pending → running → succeeded | failed(可重试→pending) | dead(重试耗尽)`。

| 字段                                                          | 说明                                                   |
| ------------------------------------------------------------- | ------------------------------------------------------ |
| `kind` / `payload`                                            | 任务类型 / jsonb 载荷（链式 job 经 payload 传下游 ID） |
| `status`                                                      | pending/running/succeeded/failed/dead                  |
| `attempts` / `max_attempts`                                   | 重试计数 / 上限（默认 3）                              |
| `idempotency_key`                                             | 幂等键（UNIQUE）                                       |
| `due_at` / `locked_by` / `locked_at` / `visibility_timeout_s` | 调度与抢占（SKIP LOCKED）                              |
| `error` / `progress`                                          | 错误 / 进度（jsonb）                                   |

索引：`idx_jobs_claimable`（`due_at` where pending）、`idx_jobs_status_kind`、`idx_jobs_visibility`。

### job_events — 任务事件流（追加）

`job_id → jobs.id`（CASCADE），字段 `level`（info/warn/error）/ `message` / `data`。

### llm_providers — LLM 提供方

| 字段                | 说明                                                 |
| ------------------- | ---------------------------------------------------- |
| `name` / `base_url` | 名称（UNIQUE）/ OpenAI 兼容地址                      |
| `api_key_encrypted` | AES-256-GCM 加密（nonce 12B ‖ ciphertext ‖ tag 16B） |
| `models`            | jsonb 数组 `[{"id","capabilities":["embedding"]}]`   |
| `is_default`        | 默认 provider 标记                                   |

### llm_usage — 用量记账（每次 LLM 调用一行）

`provider` / `model` / `purpose` / `input_tokens` / `output_tokens` / `latency_ms` / `job_id`。

### api_keys — API key

`key_hash`（sha256，UNIQUE）/ `key_prefix`（展示用前 8 字符）/ `scopes`（jsonb）/ `revoked_at` / `last_used_at`。

### admin_sessions — 管理员会话

`token_hash`（PK，sha256）/ `expires_at` / `last_used_at`。

### settings — 通用设置（键值）

`key`（PK）/ `value`（jsonb）/ `updated_at`。存 LLM 路由表（`llm_routing`）、wiki purpose 等。

## 记忆域

分层蒸馏，全程可溯源：

```text
raw_sessions (L0)  ──extract──▶  atoms (L1)
                                    │ arbitrate: 新增/去重/矛盾取代
                                    ▼
                                scenarios (L2)  ──organize 归组──▶
                                    │
                                    ▼
                              persona_aspects (L3)  ──persona 版本化──▶
```

### raw_sessions — L0 原始会话（不可变）

`agent` / `content`（jsonb，`[{speaker,text,ts}]`）/ `metadata` / `distill_status`（pending/processing/done）。

### atoms — L1 原子记忆

| 字段                         | 说明                                                                 |
| ---------------------------- | -------------------------------------------------------------------- |
| `kind`                       | preference/fact/decision/event/insight/correction/failure/convention |
| `content` / `confidence`     | 文本 / 置信度（默认 0.8）                                            |
| `source_refs`                | jsonb `[{session_id, span}]` 溯源                                    |
| `status`                     | candidate/active/superseded/archived（0011 增加 candidate）          |
| `superseded_by`              | 被哪个原子取代（矛盾取代）                                           |
| `needs_review` / `hit_count` | 待审标记 / 命中计数                                                  |
| `scenario_id`                | 归属的 L2 场景（0011 增加，NULL = 未归组）                           |
| `embedding` / `tsv`          | 向量（HNSW）/ 全文（GIN）                                            |

索引：`idx_atoms_active_kind`、`idx_atoms_status`、`idx_atoms_tsv`、`idx_atoms_embedding`、`idx_atoms_ungrouped`、`idx_atoms_candidate`、`idx_atoms_content_trgm`。

### scenarios — L2 场景知识块

`topic` / `summary` / `body` / `atom_refs`（jsonb）/ `version` / `embedding` / `tsv`。

### persona_aspects — L3 画像分面（版本化可回滚）

`aspect`（identity/preferences/skills/constraints/communication_style/goals/routines）/ `content` / `evidence_refs` / `version` / `prompt_version`。每 aspect 的当前版本 = 最大 version（UNIQUE 索引 `idx_persona_aspect_version`）。

## 知识域

```text
documents ──parse/chunk──▶ chunks ──embed──▶ 混合检索
```

### documents — 文档

`title` / `source_uri`（文件名或 URL）/ `mime` / `raw_path`（data/uploads/）/ `sha256`（UNIQUE 去重）/ `status`（pending/parsing/chunking/embedding/ready/failed）/ `error`。

### chunks — 分块

`document_id → documents.id`（CASCADE）/ `seq` / `content` / `embed_failed` / `embedding` / `tsv`；UNIQUE（document_id, seq）。索引：`idx_chunks_tsv`、`idx_chunks_embedding`、`idx_chunks_content_trgm`。

## Wiki 域

```text
wiki_sources ──ingest(两步)──▶ wiki_pages ──wikilink──▶ wiki_links
```

### wiki_sources — 原料（不可变）

`sha256`（UNIQUE）/ `raw_path` / `title` / `status`（pending/processing/ready/failed）/ `last_ingested_at`。

### wiki_pages — 页面

`slug`（UNIQUE）/ `title` / `page_type`（entity/concept/source/synthesis/comparison/queries/overview/index/log/purpose）/ `content`（Markdown）/ `frontmatter`（jsonb，含 `sources[]`）/ `origin`（llm/human）/ `version` / `embedding` / `tsv`。

### wiki_links — 链接图

`from_slug` / `to_slug`（联合 PK）/ `weight`（wikilink 边 3.0，source-overlap 边 4.0）。

### wiki_review_items — Review 系统（0012）

`kind`（create_page/deep_research/skip/flag）/ `payload` / `action` / `search_queries` / `source_id → wiki_sources.id` / `status`（open/resolved/dismissed）。

### wiki_insight_dismissals — 洞察忽略

`insight_key`（PK，`类型:slug` 稳定键）/ `created_at`。

## CodeGraph 域

### cg_projects — 项目注册表

`name`（UNIQUE）/ `path`（data/codegraph/<id>/ 工作目录）/ `source_uri`（本地路径或 git URL）/ `status`（registered/indexing/ready/error/version_mismatch）/ `stats`（jsonb {files,symbols,edges}）/ `last_synced_at`。图谱数据本身由 codegraph CLI 自管于 `data/codegraph/`，不落库。

## 数据流小结

| 流       | 路径                                                                                              |
| -------- | ------------------------------------------------------------------------------------------------- |
| 记忆写入 | `POST /memory/sessions` → `raw_sessions` → distill job 链 → `atoms`/`scenarios`/`persona_aspects` |
| 知识摄取 | `POST /knowledge/upload`（或 submit_url）→ `documents` → job（parse→chunk→embed）→ `chunks`       |
| Wiki     | `POST /wiki/ingest` → `wiki_sources` → job（analyze→generate）→ `wiki_pages` + `wiki_links`       |
| 检索     | `search` crate 单 SQL 融合 `atoms`/`scenarios`/`chunks`/`wiki_pages` 的 FTS + 向量                |
| 任务     | 一切长操作 → `jobs` → `job_events`（事件流）                                                      |
| LLM      | 每次调用 → `llm_usage`（记账）                                                                    |
