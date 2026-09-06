# 架构

> 2026-08-30 按当日源码重写。目录树为实查，依赖方向从 Cargo.toml members 与 use 关系归纳。

## Workspace 布局

```text
server/
├── Cargo.toml              # workspace（version 0.1.0，edition 2024，resolver 2）
├── migrations/             # 21 个 SQL 迁移（0001 起，含 CREATE EXTENSION vector）
└── crates/
    ├── api/                # HTTP 门面（axum + utoipa + rust-embed）——唯一二进制出口
    ├── mcp/                # MCP 适配器（rmcp Streamable HTTP）：六域渐进式发现工具面，与 HTTP 平级
    ├── core/               # 领域服务与编排（memory/project/skills/wiki_docs/unified）+ auth/state
    ├── storage/            # 仓储层：持久化模型（models）+ 领域表业务面 SQL 唯一收口（repo）
    ├── llm/                # LLM 网关：provider 管理、密钥加密、路由表、用量记账
    ├── jobs/               # 任务队列：入队/轮询/重试/死信/事件流水 + 管理面读写（admin）
    ├── distill/            # 记忆蒸馏流水线（L0→L1 提取、仲裁、组织、画像）
    ├── wiki-engine/        # LLM Wiki：摄取、分析、生成、lint、图谱、洞察
    ├── cg-bridge/          # CodeGraph 桥接（调用外部 codegraph CLI 索引代码库）
    ├── search/             # 跨域统一检索（记忆/知识/Wiki 融合排序）
    └── parsing/            # 文档解析（pdf-extract / docx-rs / 纯文本）
```

## 依赖方向（谁用谁）

```text
api ──▶ core（服务/编排）──▶ storage（仓储）──▶ DB
  ├─▶ engram-mcp（MCP 适配器）──▶ core + storage
  └─▶ 域流水线 crate（distill / wiki-engine / cg-bridge / search / llm / jobs）
distill / wiki-engine ──▶ llm（网关调用）
jobs ──▶ storage（测试）；各域 crate 经 JobQueue 用作异步执行器
```

规则（2026-09-05 分层收敛后）：
- **SQL 收口**：领域表的业务面读写唯一出现在 `engram-storage::repo`（行类型在
  `engram-storage::models`）；`core` 与 `api`/`mcp` 的 src 层零 sqlx 引用
  （core/api 仅测试夹具保留 sqlx dev-dependency 直连 PG 造数）。
- **双适配器**：HTTP（api）与 MCP（engram-mcp）平级，各自只做协议壳
  （鉴权、DTO、错误桥），业务语义都在 core；共用 `engram_core::auth::Principal`
  与 `engram_core::state::AppState`。
- **事务边界归仓储**：跨语句事务（快照+更新、purge、合并）整体封装在 repo 函数内。
- 流水线 crate（distill / wiki-engine / llm / search / jobs）按既有设计拥有
  各自管道内部的 SQL（job 处理器 / 检索只读路径 / 路由表），后续可按同模式收敛。
- 域 crate 之间不横向 import（wiki-engine 不 use distill）。

## api crate 内部（门面细图）

```text
crates/api/src/
├── main.rs                 # 启动：配置→PgPool→迁移→路由→监听
├── lib.rs                  # 组装（集成测试入口）
├── config.rs               # 环境变量解析（见 run-and-deploy.md）
├── state.rs                # AppState re-export（定义在 core::state，api/mcp 共用）
├── auth.rs                 # Bearer 中间件：ams_/amk_ 认证（Principal/SCOPES 定义在 core::auth）；
│                           #   /jobs + Accept:text/html → SPA 分流（2026-08-30 新增）
├── mcp_admin.rs            # /settings/mcp 管理端点 HTTP 壳（工具面在 engram-mcp）
├── error.rs                # ApiError（code/retryable，统一 JSON 错误体）
├── web_assets.rs           # rust-embed 托管 web/dist（编译期要求目录存在）
├── bin/openapi-dump.rs     # 导出 OpenAPI JSON 的工具二进制
└── routes/                 # 9 域路由 + mod.rs（路由表 + OpenApi 聚合；
    ├── auth_api.rs         #   新端点必须 .route() + paths() 双注册，漏一半快照测试红）
    ├── memory_api.rs       # 会话/原子/场景/画像/蒸馏 + 实体 9 端点 + purge/export/reembed
    │                       #   （编辑分权：Principal 二次校验，403 文案指路 correction）
    ├── wiki_docs_api.rs    # 文档上传/状态/重嵌入/分块检索
    ├── wiki_api.rs         # 页面/图谱/lint/洞察/提案/源数据/目的/摄取/搜索
    ├── codegraph_api.rs    # 项目注册/索引/查询
    ├── jobs_api.rs         # 任务列表/详情/事件/revive
    ├── llm_api.rs          # provider CRUD/路由表/API Key/重加密（llm scope 可管 provider）
    ├── search_api.rs       # POST /search 跨域统一检索
    └── health.rs           # /health /ready
```

## 关键机制

- **任务化异步**：LLM 相关长操作（蒸馏、嵌入、wiki 生成）入 jobs 队列异步执行，
  前端轮询任务状态；死信可 revive（attempts 计数 + job_events 流水）。
  deep_purge 任务自带两阶段语义（armed→succeeded/cancelled，job 行即审计链）。
- **蒸馏管线**（distill crate）：extract（含实体抽取，空认领也链 organize——直写原子可聚类）
  → arbitrate（ANN∪FTS 并集相似池，LLM 仲裁取代链）→ organize（聚类 + **快照收敛** step0：
  成员非 active 重算/解散，removed_texts 下传）→ consolidate（近重复合并 + 实体档案 + stale 降权）
  → persona（**清退 > 手动钉住 > 自动重写**；素材全空 + removed 命中时确定性写空版本）。
  敏感原子不进任何摘要输入；归档敏感原子 30s 防抖触发快照刷新。
- **boot 对账**：服务启动时 processing 会话一律重置 pending（进程崩溃恢复）。
- **LLM 网关**：provider 的 api_key 以主密钥（64 hex 环境变量）AES 加密存库；
  路由表按 purpose（extract/arbitrate/embed/organize/consolidate/wiki_analysis/persona/wiki_generation）
  配置 provider+model；每次调用记 llm_usage（token/延迟）。base_url 不带 /v1（拼接契约）。
- **向量检索**：chunks.embed 为 pgvector 列，语义检索走 L2 距离 + jieba 分词关键词融合。
  记忆域四层检索（l1/l2/l3/entities），reembed 修复缺失向量。
- **编辑分权**：handler 级 Principal 二次校验（非中间件）——AI 禁改语义内容，
  改动写 atom_revisions + jobs 审计行；purge 不吞审计凭证。
- **SPA 同源托管**：生产模式 rust-embed 把 web/dist 打进二进制；`/jobs` 因与 API 同路径，
  认证层按 Accept 分流（浏览器导航回 index.html，API 客户端照常 JSON）。

找具体端点看 [api.md](api.md)；找表结构看 [data-model.md](data-model.md)。
