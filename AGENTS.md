# AGENTS.md — engram 仓库工作指引

> 本仓库 = **engram**（单用户 AI 长期记忆平台，Rust 单二进制 + React SPA + PG）。
> 文档即数据：全部架构文档沉淀在 engram 自身 projects 域（`name=engram`），
> 本地仓库不留文档（README 仅 GitHub 门面）。本文件是 engram 档案索引快照，
> **权威在 engram 服务端**——文档清单变化时同步更新本段。

## 档案寻址

- **projects**：`name=engram`（id `01a0e6de-f166-7580-a3c0-dce4af7f62c6`，11 分类）
  读：`doc_search` / `doc_get`；写：`doc_add` / `doc_patch`
- **codegraph**：`name=engram`（cloud_index @ github.com/trtyr/engram）。
  查询前看 `codegraph list` freshness——`stale=true` 先本机 `codegraph sync` + 服务端 `sync`
- 位置：主开发机 MacBook Air M1 `/Volumes/trtyr_for_mac/Code/engram`；
  生产 engram.trtyr.top（腾讯云北京 Docker compose，宿主无 clone，无 IaC）
- 文档基线：**HEAD `1920f0c`**（2026-10-03 午后 P010b 日志真分页 `{logs,total}`+scope 筛 / EN-BUG-1 study learned_at 修复——四篇文档已对齐；上轮 e2cb949/P010 日志统一，迁移 67）

## 文档清单（title → category → doc_id 完整 UUID，一跳 doc_get 直达）

**总览**
- 概览与目标 `01a0e6e3-b7ad-79f0-9a8d-4dd7a68814df`
- 系统上下文（L1）`01a0e6f4-bea6-7440-90d8-9b66abaa546f`
- 术语表 `01a0e6f6-62c5-7a93-bf69-cc5617c49d66`

**架构与实现**
- 构建块视图 · 总索引（11 crates 五层依赖）`01a0e6e6-2ba2-7c20-a187-4c6eda17858a`
- 构建块 · api crate（启动序列/中间件栈）`01a0e6ea-5470-7f42-9fd3-c7063593f63a`
- 构建块 · mcp crate（十域工具面/渐进式发现/三级开关）`01a0e6e9-796a-7961-b193-bdaf3d3b7059`
- 构建块 · 蒸馏链（L0→L3/取代链/遗忘级联）`01a0e6f3-3829-7e21-a472-2350e0d8a814`
- 构建块 · wiki-engine 与 search（摄取/互链/RRF 混合检索）`01a0e6f8-134c-7fc2-bb0a-366b8cd6e990`
- 构建块 · 周边支撑（jobs/llm/parsing/cg-bridge/errors/tool-calling）`01a0e6f8-9091-7932-b211-c9b7c02b7752`
- 构建块 · wiki 维护工作流手册（agent-first）`01a0f998-78b0-7561-aec2-2c31175914a5`
- 运行时视图 · 两条端到端流程 `01a0e6f4-59c6-7e53-a314-8ca0ca970dc8`

**接口面**
- HTTP API 路由全清单（14 域穷举）`01a0e6e8-56e5-77d3-b2fe-7da9b9457677`
- 认证与 scope 分权（12 scope+:ro）`01a0e6e8-da71-7440-98b8-54a1d028aa07`
- 接口面 · 错误码全表（P006 注册表导出）`01a0f947-05d2-7a03-bb40-45ed7af65e9c`

**数据层**
- storage 与 SQL 迁移 67（HNSW/事务边界/KV/credentials AES-GCM/study 域落点/日志真分页）`01a0e6f3-d2cb-7032-b092-5bbeb0d9665c`

**前端**
- 架构与数据流（api.ts/认证/轮询/构建链）`01a0e6e7-5ac4-7c02-a5cd-24f4b5dd7bff`
- 路由与页面清单（穷举 17+3）`01a0e6ec-dc46-7b10-89c3-fca13f7090a7`

**运维/部署/测试**
- 本地宿主链（setup.sh+engramctl）`01a0e6ea-f2f4-7cf0-933d-54354e211456`
- Docker 链与数据迁移（两套口径勿混）`01a0e6eb-94d4-7291-bcc2-3f965637c2ff`
- 测试与门禁（三层体系+E2E_BASE 硬门）`01a0e6ec-098a-7531-8d5e-90a6c8cd073b`

**决策/规划/历史**
- 技术决策 ADR 轻量集（15 条）`01a0e6f5-8e7c-7dd2-bb06-913794767bd5`
- 风险与债（四维排序 22 项）`01a0e6f6-04d2-7b03-b128-29863af0e008`
- 开工记录 2026-09-28（基线 HEAD 82fdba7）`01a0e704-a36e-7651-94f9-290bc317b718`
- 更新记录 2026-10-01→02 · 夜间长任务三线（P004/P005/P006）`01a0f99b-53f1-7d30-9b61-01378442f731`
- 更新记录 2026-10-03 午后 · P010b 日志真分页 + EN-BUG-1 对齐（文档基线 1920f0c）`01a100f2-2cb8-7e81-adb0-15bff09523c2`
- 生产重部署 2026-10-03（a6beb1a→c0875eb · 86 commit · 迁移 63→66）`01a0fdc7-8a43-7603-a961-0e7a2bf29860`
- （P010 增量已并入既有篇目：数据层 63→67 / 接口面 / 前端路由 / mcp crate 12 工具位 / api crate 日志面）
- study×wiki×harness×memory 学习工作流分工手册 `01a0fba1-506b-79d0-9da0-765ac9fe9129`

## 图清单（projects 文件区，file_get 直取）

`diagram-01-l2-container-view.html`（L2 容器/11 crates 分层）· `diagram-02-deploy-topology.html`（生产+本地部署拓扑）· `diagram-03-distill-pipeline.html`（蒸馏链五阶段流水线）· `diagram-04-mcp-scope.html`（MCP 渐进式发现+scope 分权）· `diagram-05-data-model.html`（十域数据模型概览）· `diagram-06-frontend-routes.html`（前端路由地图+构建链）

## 检索配方（按意图直达）

- 架构怎么设计 → 「构建块视图 · 总索引」+ 对应构建块子篇
- 这接口是什么 → 「接口面 · HTTP API 路由全清单」；权限问题 → 「认证与 scope 分权」
- 为什么这么定 → 「技术决策（ADR 轻量集）」
- 有什么坑 → 「风险与债（四维排序）」；改前端 → 「前端 · 架构与数据流」+ 路由清单
- 部署/迁移 → 「部署与运维」两篇；记忆蒸馏原理 → 「构建块 · 蒸馏链」+ 运行时视图
- 术语不懂 → 「术语表」；数据表结构 → 「数据层」+ diagram-05

## 关键纪律（改代码前必读）

0. **规划工作面**：本地 `docs/plantree/`（P001 风险债整改线进行中，待拍板问题见其 open-questions.md）——分工约定：Mia 整理决策点并执行，trtyr 只拍板（P009 frontend-ia-observability 已 done：前端 IA 重构 + 生产 codegraph 数据根修复；P010 logs-unify 已 done：日志统一为系统唯一时间线 + MCP logs 域）

1. **门禁**：server `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`；web `pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build`
2. **本地 journey 唯一合法入口 `scripts/e2e-local.sh`**（Playwright 无 `E2E_BASE` 直接 throw——三次误删生产库的教训）
3. 改 MCP 工具面必须过 `tests/golden/mcp_surface.json` 快照；新增 action 必须登记读写分类（dispatch.rs:718 护栏）
4. 前端产物 `web/dist` 被 rust-embed 编译期嵌入——前端改动生效必须重建 dist；`assetsDir` 必须是 `'static'`（与 /assets API 前缀相撞事故）
5. 迁移只增不改（sqlx 单向）；加迁移必同步 `migrations_test.rs` 的版本断言（当前 75）与 AGENTS.md 本条版本号（P018-T007：曾漂移 12 个版本未被发现）
6. 凭据红线：token/密码只写变量名；credentials 值永不进日志/文档/记忆
