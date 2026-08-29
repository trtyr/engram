# agent-memory 项目概览

> 全栈归档（2026-08-28 生成）：覆盖 `server/`（Rust 后端）+ `web/`（React 前端）+ `deploy/` + `scripts/`。
> 后端深度文档（各域审计、Wiki 专项、plantree）在 [server/docs/](../server/docs/README.md)，本文档不重复。

## 是什么

agent-memory 是一个**单用户 AI 长期记忆平台**。核心理念「平台即工具」：平台对外暴露 HTTP API，AI（或人）拿着 API 操纵平台——存记忆、蒸馏画像、编译 Wiki、查代码图谱；人通过 Web 控制台管理浏览。

四类长期记忆资产（短期记忆不在平台范围）：

| 资产 | 说明 |
|---|---|
| Chat Memory | L0 会话 → L1 原子 → L2 场景 → L3 画像，分层蒸馏，全程可溯源 |
| Knowledge | 文档/URL 摄取 → 分块 → 嵌入 → 混合检索（中文友好） |
| Wiki | Karpathy 模式：LLM 增量维护的互链知识库（两步 ingest + lint） |
| CodeGraph | 代码知识图谱（复用 [codegraph](https://github.com/colbymchenry/codegraph) CLI，pin 1.5.0） |

## 整体形状

monorepo：Rust workspace 后端 + React SPA 前端 + Docker 单镜像交付。PostgreSQL 17（pgvector）是唯一持久化存储；所有长操作（蒸馏/摄取/ingest/同步）走 PG 任务队列异步执行。

```text
                        浏览器
                          │  SPA（web/dist，rust-embed 内嵌进后端二进制，单端口同源）
                          ▼
        ┌─────────────────────────────────────────────┐
        │  server/  agent-memory-server（Rust, axum）   │
        │  ┌─────────┐  9 组路由 / auth / OpenAPI       │
        │  │ api     │──SPA 兜底（非 API 路径→前端）     │
        │  └────┬────┘                                  │
        │       │ 只经 core 访问域服务                   │
        │  ┌────▼──────────────────────────────────┐   │
        │  │ core（memory/knowledge/wiki/cg 门面）  │   │
        │  └─┬────┬────┬────┬────┬────┬────┬───────┘   │
        │    ▼    ▼    ▼    ▼    ▼    ▼    ▼           │
        │  storage llm jobs distill search wiki-engine │
        │                                          parsing│
        └─────┬──────────┬─────────┬────────────────────┘
              │ sqlx     │ OpenAI 兼容│ 子进程 --json
              ▼          ▼            ▼
        PostgreSQL 17  LLM provider  codegraph CLI
        + pgvector     (chat/embed)  (@colbymchenry 1.5.0)
```

## 两个工作区

| 工作区 | 角色 | 技术 |
|---|---|---|
| `server/` | 后端：HTTP API + 任务系统 + 全部领域逻辑，单二进制 `agent-memory-server` | Rust 2024（10 crate workspace，axum 0.8 / sqlx 0.8 / utoipa 5） |
| `web/` | 前端：管理控制台 SPA（登录 + 七域导航） | React 19 / TypeScript 6 / Vite 8 / Tailwind 4 / pnpm |

外围：`deploy/`（docker-compose + 多阶段 Dockerfile）、`scripts/`（Python API e2e 套件 + 3 个 shell 验证脚本 + 备份脚本）、`.github/workflows/`（CI × docker × OpenAPI 类型同步；e2e compose 栈 Playwright）。

## 关键设计特征

- **分层蒸馏（Chat Memory）**：L0 会话经 `extract → arbitrate → organize → persona` 四阶段 job 链逐层蒸馏，产物记录 `prompt_version` 可归因回放；`consolidate` 去重降权。
- **任务系统**：PG 表 `jobs` + `FOR UPDATE SKIP LOCKED` 抢占队列，重试/幂等键/事件流/链式入队。
- **混合检索**：单条 SQL 融合全文（jieba 预分词 tsvector）+ 向量 ANN（pgvector HNSW）+ RRF；另有跨域统一检索 `POST /search`。
- **LLM 唯一出口**：`llm` crate provider 抽象（OpenAI 兼容），purpose 路由、用量记账、密钥 AES-256-GCM 加密落库、主密钥重加密。
- **单端口交付**：前端构建产物 `web/dist` 由 rust-embed 内嵌进后端二进制，API 与 SPA 同源，无 CORS 场景。
- **前后端类型同步**：后端 `openapi-dump` 提取 OpenAPI → `openapi-typescript` 生成 `web/src/lib/api-schema.ts`，CI 做漂移检查。

## 交付与运行

- 生产：`deploy/docker-compose.yml` 两个服务（`pgvector/pgvector:pg17` + 本地构建 app 镜像），启动即自动迁移（14 个迁移文件）。
- 开发：后端 `cargo run -p agent-memory-api`（:8080），前端 `pnpm dev`（Vite 代理 9 个前缀到后端）。
- 首次配置：Web 登录（`AGENT_MEMORY_ADMIN_PASSWORD`）→ Settings 注册 LLM provider →（可选）purpose 路由。

详见 [architecture.md](architecture.md)（模块地图）、[frontend-backend.md](frontend-backend.md)（前后端如何连通）、[run-and-deploy.md](run-and-deploy.md)（如何跑起来）、[current-state.md](current-state.md)（当前验证基线与已知问题）。
