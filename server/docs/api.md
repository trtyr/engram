# API

> 2026-09-05 从当日代码 `openapi-dump` 活体导出，共 **100 路径 / 132 方法注册**（GET 56 · POST 50 · PUT 10 · PATCH 3 · DELETE 13）。
> 认证：除 /health /ready /openapi.json /auth/login 外全部要求 `Authorization: Bearer <token>`；
> token 两种：管理员会话 `ams_…`（POST /auth/login 签发）与 API Key `amk_…`（settings 域签发，
> 八 scope：memory/wiki/codegraph/project/skills/llm/erase/cron）。
> 权威 schema 以 `cargo run -q -p engram-api --bin openapi-dump` 输出为准（前端 CI 有零漂移门禁）。

## 认证与健康

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | /auth/login | 管理员密码 → ams_ 会话（7 天） |
| GET | /health /ready | 存活/就绪探针（ready 带 migration_version） |

## memory（L0~L3 记忆域 + 实体 + 治理）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /memory/sessions | 会话列表（过滤 agent/distill_status）/ 写入新会话（turns 数组、distill auto/manual/**off（永久豁免蒸馏扫描，metadata.distill=off）**、30s 防抖自动蒸馏） |
| DELETE/GET | /memory/sessions/{id} | 详情 / 擦除（**需 erase scope**；关联原子溯源标记 erased） |
| POST | /memory/sessions/{id}/append | 追加轮次（仅 pending 会话；agent 维度；已蒸馏 400 引导开新会话） |
| POST | /memory/sessions/import | 批量导入历史对话为会话（JSONL/文本 → turns → source=import；蒸馏感知 import 过滤对方观点） |
| POST | /memory/sessions/{id}/void | 作废会话（v2 扩大语义：pending/off 蒸馏跳过；**done 会话作废时级联归档其蒸馏产物原子**——检索/context 立即失效，原文保留可审计，落 session_void_cascade 审计行） |
| POST | /memory/distill | 触发蒸馏流水线（202 + Job[]；full=true 附带 consolidate——含实体档案 + **关系回溯**：存量实体无 session 重放也抽关系，常识关系 + 记忆明确关系；空认领也链 organize——直写原子可聚类） |
| GET/POST | /memory/atoms | 原子列表（needs_review/sensitive 过滤）/ 手工补录（幂等：同 kind+content 活体返回既有；低置信自动人审） |
| PATCH | /memory/atoms/{id} | **分权**：AI 可改 sensitive/needs_review/status/superseded_by/时间；content/kind/confidence 仅用户（403 教学文案指路 correction 流） |
| GET | /memory/atoms/{id}/revisions | 编辑留痕（atom_revisions 表，append-only） |
| GET | /memory/scenarios、/{id} | 场景列表/详情 |
| GET | /memory/persona、/history | 画像分面（含 manually_edited）/ 版本历史（evidence_refs） |
| PATCH | /memory/persona | 用户编辑分面（钉住）/ 解锁 / 查询钉住态——AI 403 |
| POST | /memory/persona/rollback | 回滚到历史版本（Json body {aspect, to_version}，回滚也钉住） |
| POST | /memory/search | 四层检索（layers: l1/l2/l3/entities；max_items；reveal 敏感；no_feedback 防热度污染；from/to 时间窗过滤——occurred_at 优先 NULL fallback created_at；valid_until 过期原子 score ×0.5 降权） |
| GET | /memory/context | Agent 上下文（画像+记忆+**实体透镜**+**pending_review 代问**；no_feedback） |
| GET/POST | /memory/entities、/graph | 实体列表（kind 过滤、密度排序）/ 新建 / 共现图谱（边=同原子共现强度） |
| GET | /memory/entities/search | 圈子语义检索（token 打分，名字命中 1.0 > 摘要 0.3） |
| GET/PATCH/DELETE | /memory/entities/{id} | 详情（atoms+scenarios+neighbors+relations）/ 用户改摘要（钉住）/ 删除（?forget=true 连带归档关联活体原子） |
| GET | /memory/entities/{id}/revisions | 摘要历史版本（append-only 编辑留痕） |
| GET/POST | /memory/entities/{id}/relations | 关系列表 / 建关系（有向 5 类：member_of/located_in/works_on/part_of/related_to；同向同类型 upsert weight+1） |
| DELETE | /memory/entities/{id}/relations/{rid} | 删关系 |
| POST/DELETE | /memory/entities/{id}/atoms/{atom_id} | 挂/摘原子（幂等） |
| POST | /memory/entities/{id}/merge | 合并（loser 成墓碑释放名字槽，返回 moved 计数） |
| GET | /memory/timeline | 全局时间轴（原子/场景/实体按时间倒序合并，图谱/时间轴切换） |
| POST | /memory/entities/batch | 批量删除（**erase scope + confirm="批量删除"** 短语防误；forget 级联归档） |
| GET | /memory/entities/export | 圈子导出（实体+关系 JSON，数据主权） |
| POST | /memory/purge | **一等清空**：按 agent（2026-09-03 彻底化：该 agent 全部会话**物理删除**含 done/sensitive，产出原子归档；需 erase scope）或 deep（**仅限管理员会话**，amk_ 一律 403；两阶段 arm 5min 冷却→token 执行/cancel 后悔药 + 确认短语；deep+agent 互斥 400） |
| GET | /memory/export | 全量导出（数据主权；敏感默认排除，?include_sensitive=true 可选，响应带 sensitive_excluded 口径） |
| GET/POST | /memory/embeddings/status、/memory/reembed | 向量缺失诊断 / 重嵌修复（202 任务，fail loudly） |
| POST | /memory/rhythm/heartbeat | **节律心跳**（外部 cron 报到，落 jobs 审计行；**cron scope + via=cron 双条件**，缺任一 403——scope 是软挡（签 key 纪律），via 是显式声明防线） |
| GET | /memory/rhythm/status | 节律状态：最近心跳 + pending 会话数 + 最老积压年龄（**memory scope 可读**——AI 的健康观察线，积压暴涨=蒸馏链故障） |
| POST | /memory/distill `{via:"cron"}` | cron 通道（**需 cron scope**，AI 标 cron 403）：consolidate 走 cron-consolidate-{日桶} 幂等（同日只跑一次全量整理），extract 永不去重（扫 pending 兜底） |

## wiki·文档原料（原 knowledge，已并入 /wiki 前缀）

> 2026-09-02 合并、2026-09-05 彻底并入：knowledge 端点并入 /wiki 前缀，前端融合成一个 Wiki 页（文档/页面/图谱/人审/提案/目标）。knowledge 概念已消除——scope 并入 wiki、表改名 wiki_documents/wiki_chunks、`/knowledge/*` 别名已删。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /wiki/documents | 文档列表 / 直建文本文档 |
| POST | /wiki/upload | multipart 文件上传（pdf/docx/html/md/txt；二进制拒绝） |
| DELETE/GET | /wiki/documents/{id} | 详情 / 删除 |
| GET | /wiki/documents/{id}/chunks | 分块明细 |
| POST | /wiki/documents/{id}/re-embed | 重嵌入（换模型后补向量） |
| POST | /wiki/documents/search | 文档块语义+关键词融合检索 |

## wiki·知识网（LLM 增量织入）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /wiki/pages | 页面列表 |
| GET/PUT | /wiki/pages/{slug} | 页面读取 / 编辑（版本+1；folder 可改，/ 分隔多级路径，0025） |
| POST | /wiki/ingest | 源文本摄取（异步：分析→生成页面→建链） |
| GET | /wiki/sources、DELETE /wiki/sources/{id} | 源数据管理 |
| GET | /wiki/graph | 链接图谱（节点=页面，社区发现结果） |
| POST | /wiki/lint | 页面一致性检查 |
| GET/POST | /wiki/reviews、POST /wiki/reviews/{id}/resolve | 人审队列与裁决 |
| POST | /wiki/insights、/insights/dismiss、/insights/reset | 洞察卡片管理 |
| GET/POST | /wiki/proposals、/wiki/proposals/apply | 待审提案聚合（DISTINCT ON job_id 取 wiki_generate 最新提案事件，修前端 N+1）/ 应用结构提案 |
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

## project（项目记忆域，第五域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /projects/types | 类型模板（开发四分类/调研六分类预设） |
| GET/POST | /projects | 项目列表（?type= 筛选）/ 新建（type 决定初始分类） |
| GET/PUT/DELETE | /projects/{id} | 项目详情（本体+位置+文档）/ 编辑 / 删除（级联） |
| POST | /projects/batch-delete | 批量删除（返回 {deleted, failed}，failed=不存在的 id） |
| POST | /projects/{id}/locations | 登记位置（多主机 ip/host/os/path/purpose） |
| GET/PUT/DELETE | /projects/{id}/locations/{loc_id} | 读 / 编辑 / 删除位置 |
| POST | /projects/{id}/docs | 新增分类文档（markdown） |
| GET/PUT/DELETE | /projects/{id}/docs/{doc_id} | 读 / 编辑 / 删除文档 |

## skills（技能域，第六域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /skills | 技能列表（摘要不含正文；?q= 搜名称/描述、?tag=、?enabled= 过滤）/ 新建（slug 唯一 409；缺省从名字推导，中文名须显式传；初始态留 rev1 快照） |
| POST | /skills/import | 批量导入 SKILL.md 全文（frontmatter 容错解析 name/description/slug/tags，支持 `>-`/`|` 块标量；每条可附带 tags（如来源子目录名）与 frontmatter 合并；逐条成败互不阻断，overwrite=true 命中已有 slug 走更新） |
| GET | /skills/export | 全量导出（含正文，按 slug 排序——技能库随时整体带走） |
| GET/PUT/DELETE | /skills/{slug} | 详情（含正文）/ 编辑（语义字段变更前自动留版本快照，enabled-only 不留）/ 删除（级联删快照） |
| GET | /skills/{slug}/revisions | 版本快照列表（新→旧，保留最近 50 版） |
| POST | /skills/{slug}/revisions/{rev_id}/restore | 回滚到某版本（回滚前先快照现状，回滚本身可再撤销） |

## jobs（任务域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /jobs | 任务列表（kind/status 过滤；任意合法凭证可读——AI 轮询自己触发的任务）。⚠️ 浏览器导航（Accept: text/html）认证层分流回 SPA |
| GET | /jobs/{id}、/jobs/{id}/events | 任务详情 / 事件流水 |
| POST | /jobs/{id}/revive | 死信复活重试 |

## settings（LLM 网关与密钥域）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /settings/llm/providers | provider 列表 / 注册（**admin 或 llm scope**；base_url 不带 /v1；一个供应商一个模型一个 key——chat 与 embedding 分开注册） |
| DELETE/PUT | /settings/llm/providers/{id} | 删除（路由引用时 400）/ 更新 |
| POST | /settings/llm/providers/{id}/test | 连通性测试（按 capability 探针 chat 或 embed） |
| POST | /settings/llm/providers/re-encrypt | 主密钥轮换后全量重加密（危险操作，admin-only） |
| GET/PUT | /settings/llm/routing | purpose→provider/model 路由表（整表替换；8 purpose 三档用量） |
| POST | /settings/llm/routing/suggest | AI 路由建议（读供应商 + 8 用途调 LLM 生成建议，不落库） |
| GET | /llm/usage | 用量记账（token/延迟/用途） |
| GET/POST | /settings/api-keys | API Key 列表 / 签发（scope；明文只在创建时返回一次）——admin-only |
| POST | /settings/api-keys/{id}/revoke | 删除（admin-only；物理删除不留记录，删除后 401 走通用文案） |
| POST | /settings/api-keys/batch-revoke | 批量删除（{ids}；物理删除，返回 {revoked}） |
| GET/PUT | /settings/mcp | MCP 服务信息 / 配置更新（服务总开关 + 工具粒度开关 disabled_tools；admin-only）——工具面本体在 **POST /mcp**（Streamable HTTP JSON-RPC，非 OpenAPI 路径；复用 Bearer 认证，五域 45 工具按 scope 分权：memory 九 + project 15 + skills 八 + wiki 八 + codegraph 五；关闭时 503） |

## 错误文案三问规范（2026-08-31 起）

每个 4xx 回答三问：发生了什么（具体）/ 为什么（原因类别）/ 下一步（可执行指引）。
六处先例：删除 key（物理删除不留记录，401 走通用文案）、ISO8601 时间格式、purge 确认短语、content 编辑 403 指路 correction、
画像/实体 403 指路"由蒸馏维护"、UUID 解析。422 是 axum 纯文本（Json 提取先于鉴权）。
