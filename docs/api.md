# API

> 权威接口定义是运行时 `/openapi.json`（utoipa 从 handler 标注生成）。本文档记录 2026-08-28 从 HEAD（299025a）`openapi-dump` 提取的完整清单：**55 paths**。
> 鉴权模型、错误体契约、模块契约的详细说明见 [server/docs/api.md](../server/docs/api.md)；本文档列全量端点 + 相对该文档的增量。

## 鉴权与错误体（摘要）

- Bearer token 两类：`ams_`（管理员会话，登录颁发，7 天）全权限；`amk_`（API key，scopes 限定 `memory`/`knowledge`/`wiki`/`codegraph`）。
- 错误体统一：`{"error":{"code","message","retryable"}}`；code 稳定可编程判断（bad_request/not_found/unauthorized/forbidden/storage_unavailable/unavailable/internal）。

## 全量端点（55 paths，按域）

### 系统 / 鉴权（public）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/health` | 存活探针 |
| GET | `/ready` | 就绪探针（compose/Dockerfile HEALTHCHECK 用） |
| GET | `/openapi.json` | OpenAPI 文档 |
| POST | `/auth/login` | 管理员登录 → 会话 token |

### 任务（jobs）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/jobs` | 列出任务 |
| GET | `/jobs/{id}` | 单任务 |
| GET | `/jobs/{id}/events` | 事件流 |
| POST | `/jobs/{id}/revive` | 复活死任务 |

### LLM 设置与用量

| 方法 | 路径 | 说明 |
|---|---|---|
| POST/GET | `/settings/llm/providers` | 注册/列出 provider |
| **PUT/DELETE** | `/settings/llm/providers/{id}` | 更新/删除 provider（**新增：生命周期**） |
| **POST** | `/settings/llm/providers/re-encrypt` | 主密钥重加密全部 provider key（**新增**） |
| POST | `/settings/llm/providers/{id}/test` | 连通性测试 |
| GET/PUT | `/settings/llm/routing` | purpose 路由配置 |
| POST/GET | `/settings/api-keys` | 签发/列出 API key |
| POST | `/settings/api-keys/{id}/revoke` | 吊销 |
| GET | `/llm/usage` | 用量记账查询 |

### 检索

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/search` | **跨域统一检索**（并行 memory+knowledge+wiki，域内 RRF 归一融合，`UnifiedHit[]` 带 domain；需三 scope 至少其一） |
| POST | `/memory/search` | 记忆域混合检索 |
| POST | `/knowledge/search` | 知识域检索 |
| POST | `/wiki/search` | Wiki 检索（带 purpose，`WikiSearchResponse`） |

### 记忆（memory）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST/GET | `/memory/sessions` | 写入/列出 L0 会话 |
| GET/DELETE | `/memory/sessions/{id}` | 取/删会话 |
| POST | `/memory/distill` | 手动触发蒸馏链 |
| GET/POST | `/memory/atoms` | 列出/手建 L1 原子 |
| PATCH | `/memory/atoms/{id}` | 更新原子（治理） |
| GET | `/memory/scenarios` · `/memory/scenarios/{id}` | L2 场景列表/单条 |
| GET | `/memory/persona` · `/history` | L3 画像当前/历史 |
| POST | `/memory/persona/rollback` | 画像回滚 |
| GET | `/memory/context` | 上下文包（供 AI 注入） |

### 知识（knowledge）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST/GET | `/knowledge/documents` | 提交 URL 摄取 / 列出文档 |
| POST | `/knowledge/upload` | 上传文件摄取（multipart） |
| GET/DELETE | `/knowledge/documents/{id}` | 取/删文档 |
| GET | `/knowledge/documents/{id}/chunks` | 文档分块 |
| **POST** | `/knowledge/documents/{id}/re-embed` | 重建嵌入（**新增**） |

### Wiki

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/wiki/ingest` | 两步 ingest 入口 |
| GET | `/wiki/pages` · GET/PUT `/wiki/pages/{slug}` | 页面列表/取/写（人工纠偏） |
| GET | `/wiki/graph` | 链接图（含 communities：top_slug/size/cohesion/sparse） |
| POST | `/wiki/lint` | lint 检查 |
| POST | `/wiki/proposals/apply` | 应用提议 |
| GET/PUT | `/wiki/purpose` | purpose 配置 |
| GET | `/wiki/reviews` · POST `/wiki/reviews/{id}/resolve` | review 系统 |
| POST | `/wiki/queries/archive` | 归档查询 |
| GET | `/wiki/sources` · DELETE `/wiki/sources/{id}` | 原料列表/删除 |
| POST | `/wiki/insights` · `/dismiss` · `/reset` | 洞察生成/忽略/重置 |

### CodeGraph

| 方法 | 路径 | 说明 |
|---|---|---|
| POST/GET | `/codegraph/projects` | 注册/列出项目 |
| GET | `/codegraph/projects/{id}` | 单项目 |
| POST | `/codegraph/projects/{id}/index` · `/sync` · `/query` | 建索引/同步/查询 |

## 对外模块契约与消费的外部接口

见 [server/docs/api.md](../server/docs/api.md) 的「对外暴露的模块契约」（core/llm/jobs/search/distill/parsing/storage 的 public API）与「消费的外部接口」（PostgreSQL / LLM provider / codegraph CLI）——2026-08-28 核实仍然有效。

## 前端消费

- 客户端：`web/src/lib/api.ts`（fetch 封装 + Bearer 注入 + 401 自动清 token + `ApiError{status,code,retryable}`）；类型：`web/src/lib/api-schema.ts`（OpenAPI 生成，当前零漂移）。
- 各 feature 页通过 react-query 调上述端点；上传走 `api.upload`（multipart）。
- 详见 [frontend-backend.md](frontend-backend.md)。
