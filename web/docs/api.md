# API 面（前端视角）

> 前端是后端 76 路径（94 方法注册）的纯消费者，不自有接口。本文档写消费约定；端点全表见
> [server/docs/api.md](../../server/docs/api.md)（当日 OpenAPI 活体导出）。

## 调用约定（lib/api.ts）

```ts
api.get<T>(path) / post / put / patch / del / upload(path, file)
```

- 认证：token 在 localStorage `am_token`，自动附 `Authorization: Bearer …`
- 401：clearToken + `engram-auth-expired` 窗口事件（有 token 时才广播）→ App 回登录页。
  登录接口的密码错误不带 token，不会误触广播。
- 非 2xx：抛 `ApiError { status, code, message, retryable }`（错误体 `e.error.*` 解包）。
- 上传走 `api.upload`（FormData，不设 content-type）。

## 类型双轨

| 轨道 | 文件 | 性质 |
|---|---|---|
| 手写域类型 | `lib/api.ts`（Session/Atom/Scenario/Persona/Job/Document/…） | 页面实际消费的形状 |
| 生成类型 | `lib/api-schema.ts`（openapi-typescript，80K） | 契约防漂移 |

生成流程：`pnpm run gen:api`（OPENAPI_URL 指向运行中后端）；CI api-types job 做零漂移 diff。
手写类型与生成类型尚未合一（历史渐进），改后端 DTO 时两处都要核对。

## 特殊路径：/jobs

`GET /jobs` 与 SPA 路由 `/jobs` 同路径。后端认证层按 Accept 分流：
浏览器导航（text/html）→ index.html（SPA 正常挂载）；fetch（默认 Accept）→ JSON。
前端 fetch 不显式设 Accept，天然走 JSON 分支；壳的登录探活故意复用该端点（`/jobs?limit=1`）。

## 前端调用点分布（按域）

Dashboard：POST /search；Memory：sessions/atoms/scenarios/persona + distill + 擦除 + 检索；
Knowledge：upload/documents/chunks/re-embed/search；Wiki：pages/graph/lint/reviews/insights/
proposals/sources/purpose/ingest/search；CodeGraph：projects/index/sync/query；
Jobs：jobs/events/revive；Settings：providers/routing/api-keys/re-encrypt/test + /llm/usage；
Login：POST /auth/login；壳：/jobs?limit=1 探活 + /jobs?limit=200 徽章轮询。

历史对接审计（44 调用点零缺漏）见 [api-alignment-audit.md](api-alignment-audit.md)（2026-08-28，
其后新增调用点以上表为准）。
