# Ideas Inbox

低承诺度想法池——promote 到具体 plan 时才立项。

## 待办 / 想法

- **分工（2026-08-28 更新）**：本会话/agent 只负责后端（`server/` 目录）；`web/` 前端由用户安排的其他人负责，后端 agent 不改动、不提交 web 文件。前端如有后端配合需求（新端点/字段调整），由前端侧提出、后端在此承接。原 2026-08-27 记录（用户亲自改前端）已被此分工取代。
- **前后端契约核对（2026-08-28，intercom 对接）**：前端 50 个调用点全部对上 8090 OpenAPI，无缺口。后端超前端点（前端待接，非阻塞）：`/knowledge/documents/{id}/re-embed`、`POST /search` 跨域、`/wiki/search`（注意 `{purpose, pages}` 包裹结构）、`/wiki/purpose`、`/memory/context`、providers/{id} PUT/DELETE、`/settings/llm/providers/re-encrypt`。前端 gen:api 已改 `${OPENAPI_URL:-http://127.0.0.1:8090/openapi.json}`（8080 被 cda-agent 占用，勿回写死）。前端 api.ts 手写类型（Phase 6a 再切生成 schema），错误契约 `{code,message,retryable}` 已按此处理。
