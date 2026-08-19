# Topic — API 设计

API 是第一公民（D0001）。本文是端点全目录与契约约定；实现时 utoipa 生成 OpenAPI 为准。

## 通用约定

- **认证**：`Authorization: Bearer <api-key>`。两种主体：管理员会话（Web UI 登录颁发）+ API key（AI 客户端用，可吊销、带 scopes）。
- **错误体**：`{"error": {"code": "string", "message": "人话", "retryable": bool, "details": {}}}`；code 稳定可编程判断；message 永不含内部细节。
- **分页**：游标式 `?limit=50&cursor=...`，响应带 `next_cursor`。
- **幂等**：会入队的 POST 接受 `Idempotency-Key` 头，重复提交返回已有 job。
- **预算**：所有检索端点接受 `budget: {max_items, max_chars}`，防上下文爆炸（借鉴 TDAM）。
- **时间**：ISO8601 UTC。

## 端点目录

### 系统

| Method Path | 说明 |
|---|---|
| GET /health | 存活（无鉴权） |
| GET /ready | 就绪（DB 连通 + 迁移版本） |
| POST /auth/login | 管理员密码 → 会话 token |

### 记忆域 /memory

| Method Path | 说明 |
|---|---|
| POST /memory/sessions | 写 L0 会话（含 speaker 轮次）；可选 `distill: auto\|manual` |
| GET /memory/sessions | L0 列表（按 agent/时间过滤） |
| GET /memory/sessions/:id | L0 详情（含蒸馏状态） |
| POST /memory/distill | 手动触发蒸馏（可指定范围） |
| GET /memory/atoms | L1 列表：kind/status/时间/全文过滤 |
| POST /memory/atoms | 手工新增 L1（人审补充） |
| PATCH /memory/atoms/:id | 编辑/归档/supersede |
| GET /memory/scenarios | L2 列表/详情 |
| GET /memory/persona | L3 画像（全部 aspect + 版本） |
| GET /memory/persona/history | 画像版本历史（含 diff 溯源） |
| POST /memory/search | 分层检索 `{query, layers, budget}` |
| GET /memory/context | **一站式上下文包**：L3 + 相关 L2 + top L1，按预算裁剪。AI 冷启动首选端点 |

### 知识域 /knowledge

| Method Path | 说明 |
|---|---|
| POST /knowledge/documents | 上传文件（multipart）或 `{url}` |
| GET /knowledge/documents | 列表（status 过滤） |
| GET /knowledge/documents/:id | 详情 + 处理状态/错误 |
| DELETE /knowledge/documents/:id | 级联删 chunks/embeddings |
| GET /knowledge/documents/:id/chunks | 分块预览 |
| POST /knowledge/search | 混合检索，结果带文档引用 |

### Wiki 域 /wiki

| Method Path | 说明 |
|---|---|
| POST /wiki/ingest | `{document_id}` 触发两步 ingest（sha 去重） |
| GET /wiki/pages | 页面列表（type 过滤/全文） |
| GET /wiki/pages/:slug | 页面详情（frontmatter + md + 版本） |
| PUT /wiki/pages/:slug | 人工编辑（版本化，见 wiki-engine） |
| GET /wiki/graph | 链接图（节点+边，供前端渲染） |
| POST /wiki/lint | 触发 lint，返回报告 |
| GET /wiki/index · GET /wiki/log | index.md / log.md 内容 |

### CodeGraph 域 /codegraph

| Method Path | 说明 |
|---|---|
| POST /codegraph/projects | 注册项目（卷内路径或 git URL） |
| GET /codegraph/projects | 列表 + 状态 |
| POST /codegraph/projects/:id/sync | 触发增量同步 |
| POST /codegraph/query | `{kind: explore\|node\|search\|callers\|callees\|impact, ...}` 代理查询 |

### 任务 /jobs

| Method Path | 说明 |
|---|---|
| GET /jobs | 列表（kind/status 过滤） |
| GET /jobs/:id | 详情（attempts/error/progress） |
| GET /jobs/:id/events | 事件流（SSE） |

### 设置 /settings（管理员）

| Method Path | 说明 |
|---|---|
| GET/POST /settings/llm/providers · PATCH/DELETE .../:id | provider CRUD（key 落库前加密） |
| POST /settings/llm/providers/:id/test | 连通性测试 |
| GET/PUT /settings/llm/routing | 任务→模型路由规则 |
| GET /llm/usage | 用量统计（时间/用途/模型聚合） |
| GET/POST/DELETE /settings/api-keys | API key 管理 |

## OpenAPI 与类型流

utoipa 注解 → `/openapi.json` → 前端 openapi-typescript 生成 `web/src/lib/api-types.ts`。禁止前端手写与后端重复的类型。
