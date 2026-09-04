# API（集成索引）

全栈共一个 HTTP API，**88 路径 / 112 方法注册**（GET 41 · POST 43 · PUT 4 · PATCH 3 · DELETE 7，2026-09-03
openapi-dump 活体导出）。权威全表在 [server/docs/api.md](../server/docs/api.md)；
前端消费约定（认证、类型双轨、/jobs 分流）在 [web/docs/api.md](../web/docs/api.md)。

## 域速览

| 域 | 代表端点 | 说明 |
|---|---|---|
| auth | POST /auth/login | 管理员会话 |
| memory | /memory/sessions、/memory/distill、/memory/context | L0~L3 全链 + Agent 上下文 |
| memory·实体 | /memory/entities、/memory/entities/graph | 记忆坐标系（人物/项目/主题/群组/地点） |
| memory·治理 | /memory/purge、/memory/export、/memory/atoms/{id}/revisions | agent 彻底清场（会话物理删除）/ deep 仅管理员两阶段 / 数据主权 / 编辑留痕 |
| wiki·文档 | /wiki/documents、/wiki/upload、/wiki/documents/search | 文档→向量检索（原 knowledge，已并入 /wiki 前缀） |
| wiki·知识网 | /wiki/ingest、/wiki/pages/{slug}、/wiki/graph、/wiki/proposals | 摄取→页面→图谱；folder 目录树 + 提案聚合 |
| codegraph | /codegraph/projects/{id}/index、/query | 注册→索引→查询 |
| jobs | /jobs、/jobs/{id}/events、/jobs/{id}/revive | 任务观测与恢复 |
| settings | /settings/llm/providers、/settings/llm/routing（含 /suggest）、/settings/api-keys（含 /batch-revoke） | LLM 网关配置 |
| search | POST /search | 跨域统一检索（含实体域） |
| health | /health、/ready | 探针 |

## 编辑与破坏性操作的分权（2026-08-31 落地）

- **AI 可改**：sensitive / needs_review / status(归档) / superseded_by / 时间字段——走 amk_ key
- **仅用户可改**：原子 content/kind/confidence、画像分面、实体摘要——Web 登录态专属，
  改动写 atom_revisions 留痕 + manually_edited 钉住（蒸馏绕开，敏感清退仍优先）
- **erase scope**：会话擦除与 agent 级清场（不可逆操作与读写分权）
- **deep 清空仅限管理员**（2026-09-03 收权）：amk_ 一律 403（确认短语是公开常量防误操作，挡不住蓄意）；Web 危险区两阶段：arm（5 分钟冷却）→ token 执行 / cancel 后悔药，job 行即审计链

## 契约管理

- schema 权威源：`cargo run -q -p agent-memory-api --bin openapi-dump`
- 前端类型：`pnpm run gen:api`（openapi-typescript）
- CI api-types job 对生成物做零漂移 diff——后端改端点不重生成即红。
- **utoipa 双注册**：新端点必须同 commit 加 `.route()` 与 `mod.rs` 的 `paths()` 列表，漏一半 CI 不报但快照测试红。
