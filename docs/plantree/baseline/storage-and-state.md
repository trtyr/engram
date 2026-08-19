# Storage & State — 存储设计概要

PostgreSQL 17 + pgvector 单库。sqlx 迁移是 schema 唯一定义处。前端静态资源由后端 axum 静态服务（单端口）。

## 表清单（按域）

### Chat Memory（分层蒸馏）

```sql
raw_sessions (L0)   -- id, agent, content jsonb, metadata jsonb, created_at（不可变）
atoms (L1)          -- id, kind(preference|fact|decision|event|insight|correction|failure|convention),
                    --   content, confidence, source_refs jsonb, status(active|superseded),
                    --   superseded_by uuid, embedding vector, tsv tsvector, timestamps
scenarios (L2)      -- id, topic, summary, body, atom_refs jsonb, embedding, tsv, version, timestamps
persona_aspects (L3)-- id, aspect, content, evidence_refs jsonb, version, updated_at
```

- L1 kind 集合融合 TDAM 分类 + hermes 分类学（failure/correction/insight/preference/convention）。
- 矛盾消解：新 atom 与旧 atom 矛盾 → 旧 status=superseded、superseded_by 指向新，检索默认只取 active。

### Knowledge

```sql
documents   -- id, title, source_uri, mime, raw_path, sha256, status, error, timestamps
chunks      -- id, document_id, seq, content, embedding, tsv
```

### Wiki

```sql
wiki_pages  -- id, slug, title, page_type(entity|concept|source|synthesis|comparison|overview|index|log),
            --   content(md), frontmatter jsonb, embedding, tsv, version, timestamps
wiki_links  -- from_slug, to_slug, weight（source重叠/wikilink 两种边来源）
wiki_sources-- id, sha256, raw_path, status, last_ingested_at  -- 摄取缓存与原料
```

### CodeGraph

```sql
cg_projects -- id, name, path, status(registered|indexing|ready|error), last_synced_at, stats jsonb
```

### 系统

```sql
jobs        -- id, kind, payload jsonb, status, attempts, max_attempts, idempotency_key,
            --   error, progress, created_at, started_at, finished_at, locked_by, locked_at
job_events  -- id, job_id, ts, level, message, data jsonb（追加）
llm_providers -- id, name, base_url, api_key_encrypted, models jsonb, routing jsonb, is_default
llm_usage   -- id, provider, model, purpose, input_tokens, output_tokens, latency_ms, job_id, ts
api_keys    -- id, name, key_hash, scopes jsonb, last_used_at, created_at, revoked_at
```

## 检索基础设施

- **向量**：pgvector HNSW 索引（atoms/chunks/wiki_pages/scenarios 各自 embedding 列）。
- **全文**：tsvector + GIN。中文分词是开放问题（见 open-questions Q1），候选：应用层 jieba 预分词写入 / pg_trgm 补位 / 纯向量兜底。
- **混合**：FTS 分数 + 向量余弦，RRF（k=60）融合，按资产预算封顶条数/字符。

## 状态与文件系统

| 数据 | 位置 |
|---|---|
| 所有结构化数据 | PG（唯一真相源） |
| 原始上传文件 | 卷挂载 `data/uploads/`（DB 存路径+sha256） |
| Wiki 原料 | `data/wiki-sources/`（不可变） |
| CodeGraph 索引 | `data/codegraph/<project>/.codegraph/`（codegraph 自管） |
| 迁移 | server/migrations/，启动时自动 `sqlx migrate run` |

## 备份

`pg_dump` + `data/` 卷即全量。提供 `deploy/backup.sh`。
