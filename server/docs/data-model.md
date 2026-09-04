# 数据模型

> 2026-09-04 实查：迁移目录 28 个 SQL；运行库 public schema 业务表 27 张（另有 `_sqlx_migrations` 簿记表）。
> 权威 schema 以 `server/migrations/` 为准。

## 表清单（27 张业务表）

| 域 | 表 | 说明 |
|---|---|---|
| 认证 | admin_sessions | 管理员会话（ams_ token，7 天 TTL） |
| 认证 | api_keys | API Key（amk_ 前缀、scope、sha256 落库；删除即物理删除不留记录） |
| 记忆 | raw_sessions | L0 会话原文（content JSONB 轮次数组、distill_status 含 void、agent 维度） |
| 记忆 | atoms | L1 原子（kind/content/confidence/status/needs_review/superseded_by/sensitive/occurred_at/valid_until、source_refs 溯源） |
| 记忆 | scenarios | L2 场景（topic/summary/atom_refs/version/hit_count；快照收敛语义：成员非 active 则重算/解散） |
| 记忆 | persona_aspects | L3 画像分面（aspect 受 CHECK 约束、evidence_refs、prompt_version、**manually_edited** 钉住） |
| 记忆 | **entities** | 记忆坐标系（name+kind 唯一[活体]、kind ∈ person/project/topic/group/place、summary、merged_into 合并墓碑、manually_edited） |
| 记忆 | **atom_revisions** | 原子编辑留痕（old_content/kind/confidence、edited_by、append-only） |
| 记忆 | **atom_entities** | 原子↔实体挂链（复合主键，双向 CASCADE） |
| 记忆 | **entity_revisions** | 实体摘要编辑留痕（old_summary/edited_by、append-only） |
| 记忆 | **entity_relations** | 实体关系（from/to/rel_type 5 类、weight、source distill/manual、方向唯一索引） |
| 知识 | documents | 上传文档（title/mime/status/error、sha256 UNIQUE） |
| 知识 | chunks | 分块（seq/snippet、embed pgvector 向量列、embed_failed） |
| wiki | wiki_sources | 摄取源（sha256 UNIQUE、error） |
| wiki | wiki_pages | 页面（slug/page_type/folder/content/frontmatter/origin/version） |
| wiki | wiki_links | 页面间链接（from/to/weight） |
| wiki | wiki_review_items | 人审队列 |
| wiki | wiki_insight_dismissals | 洞察卡片 dismissing 记录 |
| codegraph | cg_projects | 注册的代码库（path/source_uri/status/stats） |
| 项目 | projects | 项目本体（name/type[dev·research]/status/categories JSONB 分类列表可增删/frontmatter） |
| 项目 | project_locations | 多主机位置（host/path/purpose，登记制纯元数据） |
| 项目 | project_docs | 分类文档（category/title/content markdown/frontmatter） |
| 任务 | jobs | 任务队列（kind/status[attempts/error/progress]；status 含 **cancelled**——deep purge 后悔药态；审计行 kind=edit_*/purge_memory） |
| 任务 | job_events | 事件流水（level/message/data） |
| LLM | llm_providers | provider（base_url 不带 /v1、models、api_key_encrypted bytea、is_default） |
| LLM | settings | 路由表等 JSONB 配置 |
| LLM | llm_usage | 用量记账（provider/model/purpose/tokens/latency） |

## 迁移史（28 个）

| 迁移 | 内容要点 |
|---|---|
| 0001 | 基线 + `CREATE EXTENSION vector`（因此 PG 必须带 pgvector） |
| 0002~0011 | 各域表逐步演进（详见文件名） |
| 0012 | wiki 对齐（页面/链接/审阅结构） |
| 0013 | scenarios.hit_count（场景命中计数） |
| 0014 | wiki_sources.error（摄取错误记录） |
| 0015 | **entities + atom_entities**（实体层：活体唯一名、合并墓碑、共现图） |
| 0016 | atoms.occurred_at/valid_until（事件时间轴）+ entities kind +**place** |
| 0017 | atoms.**sensitive**（隐私标记）+ raw_sessions distill_status +**void** |
| 0018 | **atom_revisions** + persona_aspects/entities.**manually_edited** |
| 0019 | jobs status +**cancelled**（deep purge 两阶段） |
| 0020 | **entity_revisions**（实体摘要版本链） |
| 0021 | **entity_relations**（有向类型化关系：5 类枚举 + 方向唯一索引 + weight） |
| 0022 | llm_providers **models→model_id+capability**（一个供应商一个模型一个 key，多模型拆分成行） |
| 0023 | raw_sessions.**sensitive**（会话级敏感标记，蒸馏产物自动继承） |
| 0024 | atoms 无 source_refs 的 active 残留打标 `origin=direct-write`（溯源断但可审计） |
| 0025 | wiki_pages.**folder**（Obsidian 式目录树层级，/ 分隔多级路径；蒸馏按 page_type 归文件夹） |
| 0026 | **projects + project_locations + project_docs**（项目记忆第五域：类型模板分类 + 多主机位置 + 分类文档） |
| 0027 | project_locations.**ip** + **os**（位置元数据补齐，多主机登记） |
| 0028 | projects.**name** UNIQUE + project_docs(**project_id, category, title**) UNIQUE（防同名项目/同项目同分类同名文档） |

## 数据流（写路径）

```text
会话写入 ─▶ raw_sessions ─(distill 任务)─▶ atoms ─▶ scenarios ─▶ persona_aspects
                        │                    ↕（蒸馏自动抽取）
                        └──────────────▶ entities + atom_entities
直写原子（scenario_id NULL）──(下次 full distill)──▶ organize 自然拾起聚类
敏感归档 ─(防抖 30s)─▶ organize 快照收敛 ─▶ persona 清退（removed_texts）──快照层与源同生共死
原子/画像/实体编辑 ─▶ atom_revisions 留痕 + manually_edited 钉住 + jobs 审计行
文档上传 ─▶ documents ─(parse→chunk→embed 任务)─▶ chunks(向量)
Wiki 摄取 ─▶ wiki_sources ─(analyze→generate 任务)─▶ wiki_pages ─▶ wiki_links
以上任务全部落 jobs + job_events；LLM 调用逐条落 llm_usage
```

## 数据流（读路径）

```text
/memory/context   画像 + 相关记忆 + 实体透镜 + pending_review 代问 → Agent prompt 注入
/memory/search    四层检索（l1/l2/l3/entities），RRF 融合，no_feedback 防热度污染
/knowledge/search 向量 L2 距离 + jieba 关键词融合（敏感原子默认排除，reveal 可见）
/wiki/graph       页面+链接 → 前端 sigma 渲染（社区着色）
/search           跨域统一检索（memory/knowledge/wiki/entity 四域）
/memory/export    全量导出（数据主权；敏感默认排除，include_sensitive 可选）
```

## 已知约束（写种子/测试数据时踩过）

- llm_providers.api_key_encrypted 是 **bytea**（`decode(repeat('ab',40),'hex')` 而非字符串拼接）
- persona_aspects.aspect 有 CHECK 约束（7 分面：identity/preferences/skills/constraints/communication_style/goals/routines）
- documents.sha256 / wiki_sources.sha256 UNIQUE——随机数据用 `md5(random()::text)` 防撞
- 本环境 PG 无 `gen_random_bytes()`，用 `decode(repeat(...),'hex')` 或 md5 替代
- 聚合投影布尔列用 **bool_or** 不是 max（PostgreSQL 无 max(boolean)，先例 fce571f）
- scenarios.body NOT NULL——测试 fixture 必须 `body='正文'`
- EntityDto.atom_count 是计算列——SELECT * 会 FromRow 失败，需显式投影子查询
