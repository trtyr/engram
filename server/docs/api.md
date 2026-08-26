# API

## 概览

后端是单二进制 HTTP 服务 `agent-memory-server`（`crates/api`），用 **axum 0.8** 建路由，**utoipa 5** 自动生成 OpenAPI 文档。权威接口定义以运行时 `/openapi.json` 为准（由 `ApiDoc` 从 handler 的 `#[utoipa::path]` 标注 + 类型派生生成）。

- 路由集中定义在 `server/crates/api/src/routes/mod.rs` 的 `router()`。
- 分两层：`public`（无鉴权）+ `authed`（Bearer 中间件）。
- 所有 handler 的错误统一收敛到 `ApiError`（见 [错误体契约](#错误体契约)）。

## 鉴权模型

Bearer token 两种主体（`server/crates/api/src/auth.rs`）：

| token 前缀 | 主体 | 权限 |
|---|---|---|
| `ams_` | 管理员会话（opaque token，登录颁发，7 天过期） | 全权限 |
| `amk_` | API key（签发时限定 scopes） | 按 scopes 限制 |

- 登录：`POST /auth/login`，密码 sha256 恒定时间比较后颁发会话 token（明文只返回一次）。
- API key 存 sha256 哈希 + 前缀，明文只返回一次。
- scope 值：`memory` / `knowledge` / `wiki` / `codegraph`（`SCOPES` 常量）。
- 认证成功注入 `Principal` 扩展；scope 检查在各 handler 内用 `require_scope` 做。

## 错误体契约

所有错误响应统一为：

```json
{"error": {"code": "...", "message": "...", "retryable": false, "details": null}}
```

| HTTP | code | retryable | 触发 |
|---|---|---|---|
| 400 | `bad_request` | false | 参数不合法 |
| 404 | `not_found` | false | 资源不存在 |
| 401 | `unauthorized` | false | 未认证/凭证无效 |
| 403 | `forbidden` | false | 缺 scope |
| 503 | `storage_unavailable` | **true** | 数据库故障（sqlx 错误） |
| 503 | `unavailable` | **true** | 依赖服务不可用（LLM/job） |
| 500 | `internal` | false | 未捕获内部错误（详情只进日志） |

原则：`code` 稳定可编程判断；`message` 人话且不泄漏内部细节（内部细节只进 `tracing` 日志）。

## Endpoint 清单

按域分组。路径变量用 `{...}` 表示。全部 `authed` 路由需 `Authorization: Bearer <token>`。

### 系统与鉴权

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| GET | `/health` | `health::health` | 存活探针 |
| GET | `/ready` | `health::ready` | 就绪探针（compose 依赖/健康检查用） |
| GET | `/openapi.json` | `openapi_json` | OpenAPI 文档 |
| POST | `/auth/login` | `auth_api::login_handler` | 管理员登录，颁发会话 token |

### 任务系统（jobs）

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| GET | `/jobs` | `jobs_api::list_jobs` | 列出任务 |
| GET | `/jobs/{id}` | `jobs_api::get_job` | 取单任务 |
| GET | `/jobs/{id}/events` | `jobs_api::get_job_events` | 任务事件流 |
| POST | `/jobs/{id}/revive` | `jobs_api::revive_job` | 复活死任务 |

### LLM 设置与用量

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| POST/GET | `/settings/llm/providers` | `llm_api::create_provider` / `list_providers` | 注册/列出 LLM provider |
| POST | `/settings/llm/providers/{id}/test` | `llm_api::test_provider` | 测试 provider 连通性 |
| GET/PUT | `/settings/llm/routing` | `llm_api::get_routing` / `put_routing` | purpose 路由配置 |
| POST/GET | `/settings/api-keys` | `llm_api::create_api_key_handler` / `list_api_keys` | 签发/列出 API key |
| POST | `/settings/api-keys/{id}/revoke` | `llm_api::revoke_api_key` | 吊销 API key |
| GET | `/llm/usage` | `llm_api::usage` | 用量记账查询 |

### 跨域检索

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| POST | `/search` | `search_api::search` | 统一检索：并行查 memory+knowledge+wiki，域内 rank 归一化（RRF 风格）融合返回 `UnifiedHit[]`（带 domain 标签） |

鉴权：需 `memory` / `knowledge` / `wiki` scope **至少其一**（Admin 恒通过）。请求体 `{query, limit?}`（limit 默认 20）。

### 记忆（memory）

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| POST/GET | `/memory/sessions` | `memory_api::write_session` / `list_sessions` | 写入/列出 L0 会话 |
| GET/DELETE | `/memory/sessions/{id}` | `memory_api::get_session` / `erase_session` | 取/删会话 |
| POST | `/memory/distill` | `memory_api::trigger_distill` | 手动触发蒸馏 |
| GET/POST | `/memory/atoms` | `memory_api::list_atoms` / `create_atom` | 列出/创建 L1 原子 |
| PATCH | `/memory/atoms/{id}` | `memory_api::update_atom` | 更新原子（治理） |
| GET | `/memory/scenarios` | `memory_api::list_scenarios` | 列出 L2 场景 |
| GET | `/memory/scenarios/{id}` | `memory_api::get_scenario` | 取单场景 |
| GET | `/memory/persona` | `memory_api::get_persona` | 当前 L3 画像 |
| GET | `/memory/persona/history` | `memory_api::persona_history` | 画像历史版本 |
| POST | `/memory/persona/rollback` | `memory_api::persona_rollback` | 画像回滚 |
| POST | `/memory/search` | `memory_api::search` | 混合检索 |
| GET | `/memory/context` | `memory_api::context` | 上下文包（供 AI 注入） |

### 知识（knowledge）

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| POST/GET | `/knowledge/documents` | `knowledge_api::submit_url` / `list_documents` | 提交 URL 摄取 / 列出文档 |
| POST | `/knowledge/upload` | `knowledge_api::upload` | 上传文件摄取 |
| GET/DELETE | `/knowledge/documents/{id}` | `knowledge_api::get_document` / `delete_document` | 取/删文档 |
| GET | `/knowledge/documents/{id}/chunks` | `knowledge_api::document_chunks` | 文档分块 |
| POST | `/knowledge/search` | `knowledge_api::search` | 知识检索 |

### Wiki

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| POST | `/wiki/ingest` | `wiki_api::ingest` | 两步 ingest 入口 |
| GET | `/wiki/pages` | `wiki_api::list_pages` | 列出页面 |
| GET/PUT | `/wiki/pages/{slug}` | `wiki_api::get_page` / `put_page` | 取/写页面（人工纠偏） |
| GET | `/wiki/graph` | `wiki_api::graph` | 链接图 |
| POST | `/wiki/lint` | `wiki_api::lint` | lint 检查 |
| POST | `/wiki/proposals/apply` | `wiki_api::apply_proposal` | 应用提议 |
| POST | `/wiki/search` | `wiki_api::search` | Wiki 检索 |
| GET/PUT | `/wiki/purpose` | `wiki_api::get_purpose` / `set_purpose` | purpose 配置 |
| GET | `/wiki/reviews` | `wiki_api::list_reviews` | 列出 review 项 |
| POST | `/wiki/reviews/{id}/resolve` | `wiki_api::resolve_review` | 处理 review |
| POST | `/wiki/queries/archive` | `wiki_api::archive_query` | 归档查询 |
| GET | `/wiki/sources` | `wiki_api::list_sources` | 列出原料 |
| DELETE | `/wiki/sources/{id}` | `wiki_api::delete_source` | 删除原料 |
| POST | `/wiki/insights` | `wiki_api::insights` | 生成洞察 |
| POST | `/wiki/insights/dismiss` | `wiki_api::dismiss_insight` | 忽略洞察 |
| POST | `/wiki/insights/reset` | `wiki_api::reset_insights` | 重置洞察 |

### CodeGraph

| 方法 | 路径 | handler | 说明 |
|---|---|---|---|
| POST/GET | `/codegraph/projects` | `codegraph_api::register_project` / `list_projects` | 注册/列出项目 |
| GET | `/codegraph/projects/{id}` | `codegraph_api::get_project` | 取项目 |
| POST | `/codegraph/projects/{id}/index` | `codegraph_api::index_project` | 建索引 |
| POST | `/codegraph/projects/{id}/sync` | `codegraph_api::sync_project` | 同步 |
| POST | `/codegraph/projects/{id}/query` | `codegraph_api::query` | 图谱查询 |

## 对外暴露的模块契约

除 HTTP 外，后端通过以下 crate 的 public API 暴露能力（供内部跨 crate 调用）：

- **`core`**：`MemoryService`、`KnowledgeService`、`WikiService`（re-export）、`UnifiedSearch`（跨域检索编排）、`CgBridge`（re-export）——api 层唯一入口。
- **`llm`**：`LlmProvider` trait（`chat` / `embed` / `name`）、`ProviderRegistry`、`PurposeRouter`、`KeyCipher`。chat/embed 内置熔断器与 429 `Retry-After` 退避（≤2 次重试）。
- **`jobs`**：`JobQueue`（enqueue/claim/complete/fail/emit）、`Runner`（register/start）、`JobContext`（progress/emit/enqueue_next）。
- **`search`**：`search_atoms` / `search_scenarios`（返回 `SearchHit`）、`rrf_merge`、`tsv_text` / `tsv_query` / `tsv_query_smart`（短查询 AND、长查询 OR 兜底）。
- **`distill`**：`register_handlers` / `gateway_llm` / `trigger_auto_extract`。
- **`parsing`**：`detect_format` / `parse_bytes`。
- **`storage`**：`connect_pool` / `run_migrations` / `current_version`。

（各 crate 的完整 public 类型见 [architecture.md](architecture.md) 的职责表。）

## 消费的外部接口

| 外部 | 接口 | 说明 |
|---|---|---|
| PostgreSQL | 连接串 `AGENT_MEMORY_DATABASE_URL` | 唯一持久化存储 |
| LLM provider | OpenAI 兼容 HTTP（`base_url` + key + chat/embedding 模型） | 平台所有 LLM 调用 |
| codegraph CLI | 子进程 + `--json`（`@colbymchenry/codegraph@1.5.0`） | 代码图谱建索引/查询 |
