# agent-memory 后端概览

## 是什么

agent-memory 是一个**单用户 AI 长期记忆平台**。核心理念是「平台即工具」：平台对外暴露 HTTP API，AI（或人）拿着 API 操纵平台——存记忆、蒸馏画像、编译 Wiki、查代码图谱；人通过 Web UI 管理浏览。本文档归档的是其中的 **Rust 后端**（`server/`），不含前端（`web/`）。

四类长期记忆资产（短期记忆不在平台范围）：

| 资产 | 说明 |
|---|---|
| Chat Memory | L0 会话 → L1 原子 → L2 场景 → L3 画像，分层蒸馏，全程可溯源 |
| Knowledge | 文档/URL 摄取 → 分块 → 嵌入 → 混合检索（中文友好） |
| Wiki | Karpathy 模式：LLM 增量维护的互链知识库（两步 ingest + lint） |
| CodeGraph | 代码知识图谱（复用 [codegraph](https://github.com/colbymchenry/codegraph) CLI） |

## 整体形状

后端是一个 Rust workspace（`server/`），10 个 crate 分层协作，PostgreSQL（pgvector 扩展）为唯一持久化存储，所有长操作（蒸馏/摄取/ingest/同步）都通过一张 PG 支持的任务队列异步执行。

```text
                    ┌─────────────────────────────┐
                    │  api  (HTTP 层)              │
                    │  axum 路由 + DTO + OpenAPI    │
                    │  + 鉴权 + 统一错误体          │
                    └──────────────┬──────────────┘
                                   │ 只经 core 访问域服务
                    ┌──────────────▼──────────────┐
                    │  core  (领域编排层)           │
                    │  memory / knowledge 服务      │
                    │  wiki / codegraph 门面        │
                    └──┬───┬───┬───┬───┬───┬───┬───┘
        ┌─────────────┘   │   │   │   │   │   └──────────────┐
        ▼                 ▼   ▼   ▼   ▼   ▼                  ▼
   ┌─────────┐  ┌───────┐ ┌───┐ ┌────┐ ┌──────┐ ┌────────┐ ┌─────────┐
   │ storage │  │  llm  │ │jobs│ │distill│ │search │ │ wiki-  │ │ parsing │
   │ 连接池   │  │ provider│ │队列│ │蒸馏  │ │混合检索│ │ engine │ │ pdf/docx │
   │ 迁移    │  │ 路由   │ │worker│ │L0→L3 │ │FTS+向量│ │两步ingest│ │ html/md │
   └────┬────┘  └───┬───┘ └──┬┘ └───┬──┘ └───┬──┘ └───┬────┘ └────┬────┘
        │           │        │      │        │        │           │
        └───────────┴────────┴──────┴────────┴────────┴───────────┘
                                   │ sqlx
                                   ▼
                        PostgreSQL 17 + pgvector
```

## 关键设计特征

- **分层蒸馏（Chat Memory）**：原始会话（L0）经 `extract → arbitrate → organize → persona` 四阶段 job 链，逐层蒸馏出可溯源的 L1 原子 / L2 场景 / L3 画像，另有 `consolidate` 做去重降权。每阶段产物都记录 `prompt_version`，可归因可回放。
- **任务系统**：`jobs` crate 用 PG 表 `jobs` + `FOR UPDATE SKIP LOCKED` 抢占实现队列，支持重试、幂等键、事件流、链式入队（一个 job 成功后显式入队下游）。
- **混合检索**：`search` crate 在单条 SQL 内融合全文检索（tsvector）+ 向量 ANN（pgvector HNSW）+ RRF 融合；中文用应用层 jieba 预分词，写入与查询同源。
- **LLM 唯一出口**：所有 LLM 调用走 `llm` crate 的 provider 抽象（OpenAI 兼容），支持 purpose 路由、用量记账、密钥 AES-256-GCM 加密落库。
- **Wiki 引擎**：原料不可变，LLM 增量维护互链页面（`entity/concept/source/synthesis/...` 十种页型），两步 ingest（`analysis → generation`）+ lint + 链接图 + review 系统。
- **CodeGraph 桥**：`cg-bridge` crate 包装 `@colbymchenry/codegraph` CLI（子进程 `--json`），只做注册/同步/查询代理，不解析代码。

## 交付与运行

- 后端编译为单个二进制 `agent-memory-server`（`crates/api` 的 bin）。
- Docker 单镜像交付（`deploy/Dockerfile` 多阶段构建，运行时镜像内含 Node/git 供 codegraph 使用）。
- 启动即自动执行 `server/migrations/` 全部迁移（失败快速退出）。

详见 [architecture.md](architecture.md)（模块边界与依赖方向）、[run-and-deploy.md](run-and-deploy.md)（如何跑起来）。
