# 数据模型（2026-09-08，37 迁移 / 34 业务表）

> 权威定义在 [migrations/](migrations/)；repo 层（engram-storage）是唯一 SQL 归口。

## 表清单（按域）

**memory 用户记忆**
- `raw_sessions` — L0 会话（content 轮次数组、distill_status：pending/processing/done/void、
  sensitive、metadata（distill=off 豁免、pre_void_distill 作废前状态存档））
- `atoms` — L1 原子（kind 八类 CHECK、status active/superseded/archived、superseded_by 取代链、
  sensitive、hit_count、occurred_at/valid_until、source_refs（溯源会话）、embedding、tsv）
- `atom_revisions` — 原子改写留痕
- `scenarios` — L2 场景（topic/summary/atom_refs/version/embedding）
- `persona_aspects` — L3 画像（aspect 七类 CHECK、version 递增、evidence_refs、manually_edited 钉住；
  **空 content 版本 = F4 清退退休标记**，当前画像查询排除）
- `entities` / `atom_entities` / `entity_relations` / `entity_revisions` — 实体圈子
  （kind 五类、merged_into 合并、archived_at 归档/复活、关系五类带权重）

**wiki 知识库（多库，0037 起）**
- `wiki_libraries` — 库（slug 唯一；main 主库迁移自动创建）
- `wiki_pages` — 页面（**UNIQUE(library_id, slug)**、page_type 十类 CHECK、folder 目录树、
  frontmatter jsonb（title/sources[]/via）、origin human/llm、version、embedding、tsv）
- `wiki_page_versions` — 版本快照（覆盖/删除时自动留，每 slug 50 版；删除页可重建）
- `wiki_links` — 双链边（**PK(library_id, from_slug, to_slug)**、weight 4 信号）
- `wiki_sources` — 织入原料（**UNIQUE(library_id, sha256)**、status pending/processing/ready/failed）
- `wiki_documents` / `wiki_chunks` — 文档子系统（**UNIQUE(library_id, sha256)**、分块+嵌入）
- `wiki_review_items` — 人审队列（kind 四类、status open/resolved/dismissed）
- `wiki_insight_dismissals` — 洞察屏蔽（**PK(library_id, insight_key)**）
- purpose 不占表：settings 键 `wiki_purpose:{library_id}` 每库一份

**projects 项目记忆**
- `projects`（name 唯一、type dev/research、status 四态、categories jsonb）
- `project_locations`（多主机登记：ip/host/os/path/purpose）
- `project_docs`（project_id + category + title 唯一、content）

**skills 技能**
- `skills`（slug 唯一、content、tags、enabled、source manual/import/mcp）
- `skill_revisions`（版本快照 rev 递增、origin create/update/restore、留 50 版）
- `skill_files`（附属文件：skill_id + path 唯一）

**todos 待办**
- `todos`（title ≤200、body、status open/done/archived、priority、tags GIN、due_at、
  project_hint、done_at 幂等）

**codegraph**
- `cg_projects`（name/source_uri 唯一、status、stats jsonb；索引本体在 CLI 侧 .codegraph/codegraph.db）

**平台**
- `api_keys`（amk_ key 哈希、name、scopes 九种、revoked）
- `admin_account` / `admin_sessions` — 管理员登录（PBKDF2）与设备会话
- `settings` — KV（MCP 配置、wiki_purpose:{lib}、SSRF 等）
- `jobs` / `job_events` — 任务队列与事件流（幂等键、重试/退避、审计行）
- `llm_providers` / `llm_usage` — 供应商与记账

## 迁移史（要点）

0005 memory 四层 → 0007 wiki → 0012 review/dismiss/purpose → 0014 source error →
0015/0021 实体/关系 → 0025 folder → 0028 project 唯一 → 0029/0030 文档重命名/命名清理 →
0031/0032 skills + 文件夹 → 0033 admin_account → 0034 entities 归档 → 0035 todos →
**0036 wiki 页面版本快照** → **0037 wiki 多库（wiki_libraries + 全表 library_id + 库内复合唯一）**

## 约束陷阱备忘

- 跨库后 slug/sha 唯一是**库内**复合唯一——`ON CONFLICT (slug)` 类裸写法已全部改造
  （pages/sources/documents/links/versions/insight_dismissals 均带 library_id 维度）；
- persona 空版本是清退标记不是脏数据（当前画像查询排除；删除会把已退休内容误复活）；
- todos 三元组 keyset 游标（open 旗标 + updated_at + id）——纯时间游标会在 open/done 分界丢行。
