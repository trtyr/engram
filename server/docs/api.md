# API

> 2026-09-01 从运行中服务（当日代码编译，:19180）`/openapi.json` 活体导出，共 **67 路径 / 84 方法注册**（GET 34 · POST 37 · PUT 4 · PATCH 3 · DELETE 6）。
> 认证：除 /health /ready /openapi.json /auth/login 外全部要求 `Authorization: Bearer <token>`；
> token 两种：管理员会话 `ams_…`（POST /auth/login 签发）与 API Key `amk_…`（settings 域签发，
> 六 scope：memory/knowledge/wiki/codegraph/llm/erase）。
> 权威 schema 以 `cargo run -q -p agent-memory-api --bin openapi-dump` 输出为准（前端 CI 有零漂移门禁）。

## 认证与健康

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | /auth/login | 管理员密码 → ams_ 会话（7 天） |
| GET | /health /ready | 存活/就绪探针（ready 带 migration_version） |

## memory（L0~L3 记忆域 + 实体 + 治理）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /memory/sessions | 会话列表（过滤 agent/distill_status）/ 写入新会话（turns 数组、distill auto/manual/off、30s 防抖自动蒸馏） |
| DELETE/GET | /memory/sessions/{id} | 详情 / 擦除（**需 erase scope**；关联原子溯源标记 erased） |
| POST | /memory/sessions/{id}/append | 追加轮次（仅 pending 会话；agent 维度；已蒸馏 400 引导开新会话） |
| POST | /memory/sessions/{id}/void | 作废未蒸馏会话（one-way，区别于擦除） |
| POST | /memory/distill | 触发蒸馏流水线（202 + Job[]；full=true 附带 consolidate；空认领也链 organize——直写原子可聚类） |
| GET/POST | /memory/atoms | 原子列表（needs_review/sensitive 过滤）/ 手工补录（幂等：同 kind+content 活体返回既有；低置信自动人审） |
| PATCH | /memory/atoms/{id} | **分权**：AI 可改 sensitive/needs_review/status/superseded_by/时间；content/kind/confidence 仅用户（403 教学文案指路 correction 流） |
| GET | /memory/atoms/{id}/revisions | 编辑留痕（atom_revisions 表，append-only） |
| GET | /memory/scenarios、/{id} | 场景列表/详情 |
| GET | /memory/persona、/history | 画像分面（含 manually_edited）/ 版本历史（evidence_refs） |
| PATCH | /memory/persona | 用户编辑分面（钉住）/ 解锁 / 查询钉住态——AI 403 |
| POST | /memory/persona/rollback | 回滚到历史版本（Json body {aspect, to_version}，回滚也钉住） |
| POST | /memory/search | 四层检索（layers: l1/l2/l3/entities；max_items；reveal 敏感；no_feedback 防热度污染） |
| GET | /memory/context | Agent 上下文（画像+记忆+**实体透镜**+**pending_review 代问**；no_feedback） |
| GET/POST | /memory/entities、/graph | 实体列表（kind 过滤、密度排序）/ 新建 / 共现图谱（边=同原子共现强度） |
| GET/PATCH/DELETE | /memory/entities/{id} | 详情（atoms+scenarios）/ 用户改摘要（钉住）/ 删除（?forget=true 连带归档关联活体原子） |
| POST/DELETE | /memory/entities/{id}/atoms/{atom_id} | 挂/摘原子（幂等） |
| POST | /memory/entities/{id}/merge | 合并（loser 成墓碑释放名字槽，返回 moved 计数） |
| POST | /memory/purge | **一等清空**：按 agent（可逆归档）或 deep（两阶段：arm 5min 冷却→token 执行/cancel 后悔药；**需 erase scope + 确认短语"清空记忆库"**；deep+agent 互斥 400） |
| GET | /memory/export | 全量导出（数据主权；敏感默认排除，?include_sensitive=true 可选，响应带 sensitive_excluded 口径） |
| GET/POST | /memory/embeddings/status、/memory/reembed | 向量缺失诊断 / 重嵌修复（202 任务，fail loudly） |

## knowledge（知识库域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /knowledge/documents | 文档列表 / 直建文本文档 |
| POST | /knowledge/upload | multipart 文件上传（pdf/docx/html/md/txt；二进制拒绝） |
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
| POST | /wiki/insights、/insights/dismiss、/insights/reset | 洞察卡片管理 |
| POST | /wiki/proposals/apply | 应用结构提案 |
| POST | /wiki/queries/archive | 查询归档 |
| GET/PUT | /wiki/purpose | Wiki 目的（goals/scope/key_questions） |
| POST | /wiki/search | 目的导向 Wiki 检索 |

## codegraph（代码图谱域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /codegraph/projects | 项目列表 / 注册（git clone 或本地路径） |
| GET | /codegraph/projects/{id} | 项目详情与统计 |
| POST | /codegraph/projects/{id}/index、/sync | 触发 codegraph CLI 索引（异步）/ 增量同步 |
| POST | /codegraph/projects/{id}/query | 结构化查询（符号/调用关系） |

## jobs（任务域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /jobs | 任务列表（kind/status 过滤；任意合法凭证可读——AI 轮询自己触发的任务）。⚠️ 浏览器导航（Accept: text/html）认证层分流回 SPA |
| GET | /jobs/{id}、/jobs/{id}/events | 任务详情 / 事件流水 |
| POST | /jobs/{id}/revive | 死信复活重试 |

## settings（LLM 网关与密钥域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /settings/llm/providers | provider 列表 / 注册（**admin 或 llm scope**；base_url 不带 /v1；密钥服务端加密） |
| DELETE/PUT | /settings/llm/providers/{id} | 删除（路由引用时 400）/ 更新 |
| POST | /settings/llm/providers/{id}/test | 连通性测试（chat+embed 探针） |
| POST | /settings/llm/providers/re-encrypt | 主密钥轮换后全量重加密（危险操作，admin-only） |
| GET/PUT | /settings/llm/routing | purpose→provider/model 路由表（整表替换；8 purpose 三档用量） |
| GET | /llm/usage | 用量记账（token/延迟/用途） |
| GET/POST | /settings/api-keys | API Key 列表 / 签发（scope；明文只在创建时返回一次）——admin-only |
| POST | /settings/api-keys/{id}/revoke | 吊销（admin-only；吊销后 401 文案区分"已撤销"） |

## 错误文案三问规范（2026-08-31 起）

每个 4xx 回答三问：发生了什么（具体）/ 为什么（原因类别）/ 下一步（可执行指引）。
六处先例：撤销 key 区分、ISO8601 时间格式、purge 确认短语、content 编辑 403 指路 correction、
画像/实体 403 指路"由蒸馏维护"、UUID 解析。422 是 axum 纯文本（Json 提取先于鉴权）。
