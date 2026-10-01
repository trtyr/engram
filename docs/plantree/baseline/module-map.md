# 模块键（Affected Modules 用）

> Plan 元数据引用这些稳定键；详细职责见档案「构建块视图 · 总索引」。

| 键 | 内容 | 层 |
|---|---|---|
| `server/api` | crates/api — HTTP 入口/中间件/迁移端点/静态资源 | L4 |
| `server/mcp` | crates/mcp — MCP 十域工具面/渐进式发现/三级开关 | L4 |
| `server/core` | crates/core — 领域聚合/scope 判定源/transfer 迁移包 | L3 |
| `server/distill` | crates/distill — L0→L3 蒸馏管线/rhythm | L2 |
| `server/wiki-engine` | crates/wiki-engine — 摄取/互链/lint/晋升 | L2 |
| `server/cg-bridge` | crates/cg-bridge — codegraph CLI 桥 | L2 |
| `server/jobs` | crates/jobs — PG 队列 | L1 |
| `server/llm` | crates/llm — 供应商路由/KeyCipher | L1 |
| `server/search` | crates/search — RRF 混合检索 | L1 |
| `server/storage` | crates/storage — repo 层/迁移执行 | L0 |
| `server/parsing` | crates/parsing — pdf/docx/html→文本 | L0 |
| `migrations` | server/migrations/*.sql（63 个，只增不改） | 基础 |
| `web` | web/ — React 19 SPA（17 工作区路由） | 前端 |
| `deploy` | deploy/ — Dockerfile/compose/.env.example | 部署 |
| `scripts` | scripts/ — setup.sh/engramctl/backup.sh/e2e/benchmark | 工具 |
| `ci` | .github/workflows/ — ci.yml + e2e.yml | 工程化 |
| `docs-repo` | README.md / AGENTS.md / docs/plantree/（本树） | 仓库面 |
