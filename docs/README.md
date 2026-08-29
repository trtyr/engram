# agent-memory 文档归档（全栈）

这是 `agent-memory` 仓库**全项目**（server + web + deploy + scripts）的完整文档归档，由 `project-init` 在 2026-08-28 生成。后端深度文档（四份域审计、Wiki 专项、plantree 规划树）在 [server/docs/](../server/docs/README.md)，与本文档互补不重复。

30 秒了解项目 → [overview.md](overview.md)。

## 文档索引

| 文档 | 内容 | 何时读 |
|---|---|---|
| [overview.md](overview.md) | 项目是什么、四类记忆资产、全栈形状图、关键设计特征 | 想 30 秒了解全貌时 |
| [architecture.md](architecture.md) | 仓库布局、server 10 crate 边界与依赖方向、web src 地图、前后端边界、scripts；含对 server/docs 的增量修正（14 迁移、9 路由组等） | 想找某段代码在哪时 |
| [tech-stack.md](tech-stack.md) | 两端语言/框架/库及精确版本（Cargo.lock + pnpm-lock 核实）、工具链命令对照、CI/CD | 想查依赖版本时 |
| [api.md](api.md) | 全量 55 endpoints（openapi-dump 实提）、鉴权与错误体摘要、相对 server/docs/api.md 的增量 | 想查接口/写客户端时 |
| [data-model.md](data-model.md) | 19 张表总览、0013/0014 增量（scenarios.hit_count、wiki_sources.error）、数据流 | 想查数据库结构时 |
| [frontend-backend.md](frontend-backend.md) | 前后端如何连通：Vite 代理 / rust-embed 同源 / OpenAPI 类型管线 / 鉴权流 | 想动对接层时 |
| [run-and-deploy.md](run-and-deploy.md) | 本地开发两端命令（实测标注 ✅/⚠️）、环境变量、Docker 部署、备份、e2e 与验证脚本 | 想跑起来/部署时 |
| [conventions.md](conventions.md) | 后端约定摘要 + 前端约定（pnpm/目录/测试共置）、git 工作流、CI 过程与现状 | 想贡献代码前 |
| [current-state.md](current-state.md) | 验证基线（真实 exit code）、CI 现状、7 条开放项（含推送前必修的 pnpm 破坏面） | 想知道「现在能跑吗、有什么坑」时——**先读这个** |

## 跳过项

无——全栈归档 9 份文档全部成文（backend-only 时代跳过的 frontend-backend.md 本次补齐）。

## 一句话现状

本地全门禁绿（后端 100 测试、前端 21 测试、类型零漂移、e2e 无 provider 全旅程 PASS），08-28 基线的 7 条开放项已全部处置——详见 [current-state.md](current-state.md)。
