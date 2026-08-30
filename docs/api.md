# API（集成索引）

全栈共一个 HTTP API，**55 路径**（GET 26 / POST 24 / PATCH 1 / DELETE 4，2026-08-30 运行中服务
OpenAPI 活体导出）。权威全表在 [server/docs/api.md](../server/docs/api.md)；
前端消费约定（认证、类型双轨、/jobs 分流）在 [web/docs/api.md](../web/docs/api.md)。

## 域速览

| 域 | 代表端点 | 说明 |
|---|---|---|
| auth | POST /auth/login | 管理员会话 |
| memory | /memory/sessions、/memory/distill、/memory/context | L0~L3 全链 + Agent 上下文 |
| knowledge | /knowledge/upload、/knowledge/search | 文档→向量检索 |
| wiki | /wiki/ingest、/wiki/pages/{slug}、/wiki/graph | 摄取→页面→图谱 |
| codegraph | /codegraph/projects/{id}/index、/query | 注册→索引→查询 |
| jobs | /jobs、/jobs/{id}/events、/jobs/{id}/revive | 任务观测与恢复 |
| settings | /settings/llm/providers、/settings/llm/routing、/settings/api-keys | LLM 网关配置 |
| search | POST /search | 跨域统一检索 |
| health | /health、/ready | 探针 |

## 契约管理

- schema 权威源：`cargo run -q -p agent-memory-api --bin openapi-dump`
- 前端类型：`pnpm run gen:api`（openapi-typescript）
- CI api-types job 对生成物做零漂移 diff——后端改端点不重生成即红。
