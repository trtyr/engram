# 数据模型（前端状态视角）

> 前端无自持久化（唯一本地存储见文末）；全部领域数据来自后端。类型定义在 `lib/api.ts`。

## 核心域类型（手写，页面消费形状）

| 类型 | 关键字段 | 消费页面 |
|---|---|---|
| Session | content: {speaker,text,ts?}[]、distill_status、agent | Memory |
| Atom | kind、confidence、needs_review、superseded_by、source_refs（溯源，含 erased） | Memory |
| Scenario | topic、atom_refs、version、hit_count | Memory |
| Persona | aspect、evidence_refs、prompt_version | Memory |
| Job / JobEvent | kind、status、attempts、error / level、message | Jobs、壳徽章 |
| Document / ChunkHit | status（pending→parsing→chunking→embedding→ready/failed）/ score | Knowledge |
| WikiPage | slug、page_type、frontmatter、origin、version | Wiki |
| GraphDto | nodes/edges/communities（社区发现） | Wiki 图谱 |
| Provider / UsageRow / ApiKey | is_default、warning / tokens、latency / scopes | Settings |
| UnifiedHit / SearchResponse | domain、score、snippet | Dashboard、命令面板 |
| Purpose / WikiSearchResponse | goals/scope/key_questions | Wiki 目的 |

## 状态管理

- 无全局 store；页面级 `useState` + `@tanstack/react-query`（部分页面）+ 路由参数（如 `/wiki?page=slug`）。
- 壳级状态：`authed`（登录守卫）、`collapsed`（侧栏）、`openedAt`（命令面板，派生自 locationKey）。
- 轮询：Knowledge 状态机 3s（有待处理文档时）；壳徽章 10s（useSystemStatus，页面隐藏跳过）。

## localStorage 键（全部本地，无跨设备）

| 键 | 值 | 语义 |
|---|---|---|
| am_token | ams_…/amk_… | 登录凭证（401 时清除） |
| engram-theme | light/dark | 手动主题覆盖（缺省=系统跟随） |
| engram-sidebar | collapsed/expanded | 侧栏收缩态 |

## 状态机（页面内）

```text
文档: pending → parsing → chunking → embedding → ready | failed（error 列展示）
任务: pending → running → succeeded | failed（attempts≥3 → dead，可 revive）
蒸馏: 会话 distill_status: pending → processing → done|failed；管线条据此脉冲
会话: active → archived；擦除后关联原子 source_refs[].erased=true（不可逆）
```
