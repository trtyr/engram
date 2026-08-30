# API

> 2026-08-30 从运行中服务（当日代码编译，:19180）`/openapi.json` 活体导出，共 **55 路径**。
> 认证：除 /health /ready /openapi.json /auth/login 外全部要求 `Authorization: Bearer <token>`；
> token 两种：管理员会话 `ams_…`（POST /auth/login 签发）与 API Key `amk_…`（settings 域签发，带 scope）。
> 权威 schema 以 `cargo run -q -p agent-memory-api --bin openapi-dump` 输出为准（前端 CI 有零漂移门禁）。

## 认证与健康

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | /auth/login | 管理员密码 → ams_ 会话 |
| GET | /health | 存活探针 |
| GET | /ready | 就绪探针（依赖检查） |

## memory（L0~L3 记忆域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /memory/sessions | 会话列表（可过滤 agent/distill_status）/ 写入新会话 |
| DELETE/GET | /memory/sessions/{id} | 会话详情 / 擦除（关联原子溯源标记 erased） |
| POST | /memory/distill | 触发蒸馏流水线（异步任务） |
| GET/POST | /memory/atoms | 原子列表 / 手工补录原子 |
| PATCH | /memory/atoms/{id} | 原子更新（人审、状态流转） |
| GET | /memory/scenarios、/memory/scenarios/{id} | 场景列表/详情 |
| GET | /memory/persona、/memory/persona/history | 画像分面 / 版本历史 |
| POST | /memory/persona/rollback | 画像回滚到历史版本 |
| POST | /memory/search | 记忆域语义检索 |
| GET | /memory/context | Agent 上下文组装（画像+相关记忆，供 prompt 注入） |

## knowledge（知识库域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /knowledge/documents | 文档列表 / 直建文本文档 |
| POST | /knowledge/upload | multipart 文件上传（pdf/docx/md/txt） |
| DELETE/GET | /knowledge/documents/{id} | 详情 / 删除 |
| GET | /knowledge/documents/{id}/chunks | 分块明细 |
| POST | /knowledge/documents/{id}/re-embed | 重嵌入（换模型后补向量） |
| POST | /knowledge/search | 语义+关键词融合检索 |

## wiki（LLM Wiki 域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /wiki/pages | 页面列表 |
| GET/PUT | /wiki/pages/{slug} | 页面读取 / 编辑（版本+1） |
| POST | /wiki/ingest | 源文本摄取（异步：分析→生成页面→建链） |
| GET | /wiki/sources、DELETE /wiki/sources/{id} | 源数据管理 |
| GET | /wiki/graph | 链接图谱（节点=页面，社区发现结果） |
| POST | /wiki/lint | 页面一致性检查 |
| GET/POST | /wiki/reviews、POST /wiki/reviews/{id}/resolve | 人审队列与裁决 |
| GET | /wiki/insights、/insights/dismiss、/insights/reset | 洞察卡片管理 |
| POST | /wiki/proposals/apply | 应用结构提案 |
| POST | /wiki/queries/archive | 查询归档 |
| GET/PUT | /wiki/purpose | Wiki 目的（goals/scope/key_questions） |
| POST | /wiki/search | 目的导向 Wiki 检索 |

## codegraph（代码图谱域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /codegraph/projects | 项目列表 / 注册（git clone 或本地路径） |
| GET | /codegraph/projects/{id} | 项目详情与统计 |
| POST | /codegraph/projects/{id}/index | 触发 codegraph CLI 索引（异步） |
| POST | /codegraph/projects/{id}/sync | 增量同步 |
| POST | /codegraph/projects/{id}/query | 结构化查询（符号/调用关系） |

## jobs（任务域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /jobs | 任务列表（kind/status 过滤）。⚠️ 前端 SPA 同路径：浏览器导航（Accept: text/html）在认证层分流回 index.html，API 客户端照常 JSON |
| GET | /jobs/{id} | 任务详情 |
| GET | /jobs/{id}/events | 事件流水（limit 参数） |
| POST | /jobs/{id}/revive | 死信复活重试 |

## settings（LLM 网关与密钥域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /settings/llm/providers | provider 列表 / 注册（密钥服务端加密） |
| DELETE/PUT | /settings/llm/providers/{id} | 删除 / 更新 |
| POST | /settings/llm/providers/{id}/test | 连通性测试 |
| POST | /settings/llm/providers/re-encrypt | 主密钥轮换后全量重加密（危险操作） |
| GET/PUT | /settings/llm/routing | purpose→provider/model 路由表 |
| GET | /llm/usage | 用量记账（token/延迟/用途） |
| GET/POST | /settings/api-keys | API Key 列表 / 签发（scope） |
| POST | /settings/api-keys/{id}/revoke | 吊销 |

## search（跨域统一检索）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | /search | 融合检索：memory + knowledge + wiki 一次查询，统一 score 排序 |
