# 数据模型

> schema 唯一定义在 `server/migrations/`（**14 个迁移**，启动时 sqlx 自动执行）。
> 完整字段级文档见 [server/docs/data-model.md](../server/docs/data-model.md)（基于前 12 个迁移）；本文档记录全量表清单 + 2026-08-28 之后的增量（0013/0014）。

## 约定

所有 ID 用 uuid（v7），时间戳 `timestamptz`，embedding 统一 **1024 维**（bge-m3），全文 `tsv`（tsvector）由应用层 jieba 预分词维护（写入与查询同源）。扩展：`vector`（0001，pgvector）、`pg_trgm`（0009，模糊匹配）。

## 表总览（19 张）

| 域 | 表 | 迁移 |
|---|---|---|
| 系统 | `jobs` / `job_events` | 0002 |
| 系统 | `llm_providers` / `llm_usage` | 0003 |
| 系统 | `api_keys` / `admin_sessions` | 0004 |
| 系统 | `settings`（键值：llm_routing、wiki purpose 等） | 0010 |
| 记忆 | `raw_sessions` / `atoms` / `scenarios` / `persona_aspects` | 0005（0011 扩充 atoms） |
| 知识 | `documents` / `chunks` | 0006 |
| Wiki | `wiki_sources` / `wiki_pages` / `wiki_links` | 0007 |
| Wiki | `wiki_review_items` / `wiki_insight_dismissals` | 0012 |
| CodeGraph | `cg_projects` | 0008 |

## 相对 server/docs/data-model.md 的增量（0013 / 0014）

| 迁移 | 变更 | 语义 |
|---|---|---|
| **0013** | `scenarios` 加 `hit_count int NOT NULL DEFAULT 0` | 与 `atoms.hit_count` 同语义：检索命中回写（B9）；consolidate stale 降权依据 |
| **0014** | `wiki_sources` 加 `error text` | analyze/generate Permanent 失败原因落库（W4），不再只有 status=failed |

## 分层蒸馏数据流（记忆域）

```text
raw_sessions (L0, 不可变)
  ──extract──▶ atoms (L1)          ──arbitrate：新增/去重/矛盾取代──
  ──organize──▶ scenarios (L2)     ──persona 版本化──▶ persona_aspects (L3)
```

每层产物带溯源（`source_refs` / `atom_refs` / `evidence_refs`）与 `prompt_version`，L3 每 aspect 按 version 可回滚。

## 摄取数据流

| 域 | 流 |
|---|---|
| 知识 | `documents`（sha256 去重，status: pending→parsing→chunking→embedding→ready/failed）→ job 链 → `chunks`（UNIQUE(document_id, seq)） |
| Wiki | `wiki_sources`（sha256 去重，不可变）→ 两步 job（analyze→generate）→ `wiki_pages`（slug 唯一，十种 page_type）+ `wiki_links`（wikilink 边 3.0 / source-overlap 边 4.0） |
| CodeGraph | `cg_projects` 仅注册表；图谱数据由 codegraph CLI 自管于 `data/codegraph/`，不落库 |

## 检索与向量

`atoms` / `scenarios` / `chunks` / `wiki_pages` 四表带 `embedding`（HNSW）+ `tsv`（GIN）；`search` crate 单 SQL 融合 FTS + ANN + RRF（RRF_K=60）。`atoms.content` / `chunks.content` 另有 pg_trgm 索引做子串匹配。

任务一切长操作落 `jobs`（状态机 pending→running→succeeded|failed|dead，SKIP LOCKED 抢占，`job_events` 追加事件）；LLM 每次调用落 `llm_usage` 一行（provider/model/purpose/tokens/latency/job_id）。
