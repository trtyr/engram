# 数据模型

> 2026-08-30 实查：迁移目录 14 个 SQL；运行库 public schema 业务表 19 张（另有 `_sqlx_migrations` 簿记表）。
> 权威 schema 以 `server/migrations/` 为准。

## 表清单（19 张业务表）

| 域 | 表 | 说明 |
|---|---|---|
| 认证 | admin_sessions | 管理员会话（ams_ token） |
| 认证 | api_keys | API Key（amk_ 前缀、scope、吊销时间） |
| 记忆 | raw_sessions | L0 会话原文（content JSONB 轮次数组、distill_status） |
| 记忆 | atoms | L1 原子（kind/content/confidence/status/needs_review/superseded_by、source_refs 溯源） |
| 记忆 | scenarios | L2 场景（topic/summary/atom_refs/version） |
| 记忆 | persona_aspects | L3 画像分面（aspect 受 CHECK 约束、evidence_refs、prompt_version） |
| 知识 | documents | 上传文档（title/mime/status/error、sha256 UNIQUE） |
| 知识 | chunks | 分块（seq/snippet、embed pgvector 向量列、embed_failed） |
| wiki | wiki_sources | 摄取源（sha256 UNIQUE、error，0014 新增列） |
| wiki | wiki_pages | 页面（slug/page_type/content/frontmatter/origin/version） |
| wiki | wiki_links | 页面间链接（from/to/weight） |
| wiki | wiki_review_items | 人审队列 |
| wiki | wiki_insight_dismissals | 洞察卡片 dismissing 记录 |
| codegraph | cg_projects | 注册的代码库（path/source_uri/status/stats） |
| 任务 | jobs | 任务队列（kind/status/attempts/error/progress） |
| 任务 | job_events | 事件流水（level/message/data） |
| LLM | llm_providers | provider（base_url/models、api_key_encrypted bytea、is_default） |
| LLM | settings | 路由表等 JSONB 配置 |
| LLM | llm_usage | 用量记账（provider/model/purpose/tokens/latency） |

## 迁移史（14 个）

| 迁移 | 内容要点 |
|---|---|
| 0001 | 基线 + `CREATE EXTENSION vector`（因此 PG 必须带 pgvector） |
| 0002~0011 | 各域表逐步演进（详见文件名） |
| 0012 | wiki 对齐（页面/链接/审阅结构） |
| 0013 | scenarios.hit_count（场景命中计数） |
| 0014 | wiki_sources.error（摄取错误记录） |

## 数据流（写路径）

```text
会话写入 ─▶ raw_sessions ─(distill 任务)─▶ atoms ─▶ scenarios ─▶ persona_aspects
文档上传 ─▶ documents ─(parse→chunk→embed 任务)─▶ chunks(向量)
Wiki 摄取 ─▶ wiki_sources ─(analyze→generate 任务)─▶ wiki_pages ─▶ wiki_links
以上任务全部落 jobs + job_events；LLM 调用逐条落 llm_usage
```

## 数据流（读路径）

```text
/memory/context   画像 + 相关记忆 → Agent prompt 注入
/knowledge/search 向量 L2 距离 + jieba 关键词融合
/wiki/graph       页面+链接 → 前端 sigma 渲染（社区着色）
/search           三域融合统一排序
```

## 已知约束（写种子/测试数据时踩过）

- llm_providers.api_key_encrypted 是 **bytea**（`decode(repeat('ab',40),'hex')` 而非字符串拼接）
- persona_aspects.aspect 有 CHECK 约束（如 `work_style` 不合法，用 `routines`）
- documents.sha256 / wiki_sources.sha256 UNIQUE——随机数据用 `md5(random()::text)` 防撞
- 本环境 PG 无 `gen_random_bytes()`，用 `decode(repeat(...),'hex')` 或 md5 替代
