# 数据模型（前端状态视角）

> 前端无自持久化（唯一本地存储见文末）；全部领域数据来自后端。类型定义在 `lib/api.ts`。

## 核心域类型（手写，页面消费形状）

| 类型 | 关键字段 | 消费页面 |
|---|---|---|
| Session | content: {speaker,text,ts?}[]、distill_status（含 void）、agent | Memory |
| Atom | kind、confidence、needs_review、superseded_by、**sensitive**、**occurred_at/valid_until**、source_refs（溯源，含 erased） | Memory、圈子 |
| AtomRevision | old_content/old_kind/old_confidence、edited_by、created_at | 原子历史抽屉 |
| Scenario | topic、atom_refs、version、hit_count | Memory |
| Persona | aspect、evidence_refs、prompt_version、**manually_edited** | Memory 画像 |
| **Entity** | name、kind（5 种：person/project/topic/group/place）、summary、atom_count、**manually_edited**；详情带 neighbors（共现邻居）+ relations | 圈子 |
| EntityRevision | old_summary、edited_by、created_at | 实体历史抽屉 |
| EntityRelation | from_id/to_id、rel_type（5 类：member_of/located_in/works_on/part_of/related_to）、weight、source（distill/manual） | 圈子关系 |
| EntityGraph | nodes + edges（共现权重）+ relations（类型化有向关系） | 圈子图谱 |
| Job / JobEvent | kind、status（含 **cancelled**）、attempts、error / level、message | Jobs、壳徽章 |
| Document / ChunkHit | status（pending→parsing→chunking→embedding→ready/failed）/ score | Wiki 文档（原 Knowledge） |
| WikiPage | slug、page_type、frontmatter、origin、version | Wiki |
| GraphDto | nodes/edges/communities（社区发现） | Wiki 图谱 |
| Provider / UsageRow / ApiKey | model_id、capability、is_default、warning / tokens、latency / scopes | Settings |
| UnifiedHit / SearchResponse | domain、score、snippet | Dashboard、命令面板 |
| Purpose / WikiSearchResponse | goals/scope/key_questions | Wiki 目的 |

## 状态管理

- 无全局 store；页面级 `useState` + `@tanstack/react-query`（部分页面）+ 路由参数（如 `/wiki?page=slug`）。
- 壳级状态：`authed`（登录守卫）、`collapsed`（侧栏）、`openedAt`（命令面板，派生自 locationKey）。
- 轮询：Wiki 文档状态机 3s（有待处理文档时）；壳徽章 10s（useSystemStatus，页面隐藏跳过）。

## localStorage 键（全部本地，无跨设备）

| 键 | 值 | 语义 |
|---|---|---|
| am_token | ams_…/amk_… | 登录凭证（401 时清除） |
| engram-theme | light/dark | 手动主题覆盖（缺省=系统跟随） |
| engram-sidebar | collapsed/expanded | 侧栏收缩态 |

## 状态机（页面内）

```text
文档: pending → parsing → chunking → embedding → ready | failed（error 列展示）
任务: pending → running → succeeded | failed（attempts≥3 → dead，可 revive）| deep_purge: armed → succeeded/cancelled
蒸馏: 会话 distill_status: pending → processing → done|failed|void；tab 脉冲只认 processing
会话: active → archived；擦除后关联原子 source_refs[].erased=true（不可逆）
```
