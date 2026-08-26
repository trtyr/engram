# agent-memory 后端文档归档

这是 `agent-memory` 仓库 **Rust 后端（`server/`）** 的完整文档归档，由 `project-init` 在 2026-08-26 生成。范围：backend-only（前端 `web/` 不在内）。

## 文档索引

| 文档 | 内容 | 何时读 |
|---|---|---|
| [overview.md](overview.md) | 项目是什么、四类记忆资产、整体形状图、关键设计特征 | 想 30 秒了解全貌时 |
| [architecture.md](architecture.md) | 目录树、10 个 crate 的模块边界、依赖方向、每 crate 职责、启动装配、数据流 | 想找某段代码在哪个 crate 时 |
| [tech-stack.md](tech-stack.md) | 语言、框架、关键库及精确版本（来自 Cargo.toml/lock）、工具链 | 想查依赖版本时 |
| [api.md](api.md) | 全部 HTTP endpoint、鉴权模型、错误体契约、模块契约、外部接口 | 想查接口/写客户端时 |
| [data-model.md](data-model.md) | 19 张表的字段/关系/索引、分层蒸馏与各域数据流 | 想查数据库结构时 |
| [wiki/](wiki/README.md) | Wiki 模块专项文档：两步 ingest、数据模型、链接图/相关性/洞察、lint/review/purpose/级联删除、API | 想深入 Wiki 模块时 |
| [run-and-deploy.md](run-and-deploy.md) | 本地开发命令、环境变量、Docker 部署、健康检查、验证脚本 | 想跑起来/部署时 |
| [conventions.md](conventions.md) | 代码风格、错误/日志/sqlx/提示词约定、模块边界规则、git 工作流、CI | 想贡献代码前 |
| [current-state.md](current-state.md) | 验证基线（真实命令与 exit code）、未提交更改、开放项/已知问题 | 想知道「现在能跑吗、有什么坑」时 |

## 跳过项

- **frontend-backend.md**（前后端连接）——本次 scope 为 **backend-only**，前端 `web/` 未文档化，故跳过。若需了解前后端如何连通（dev 代理 / API base URL / OpenAPI 类型同步），参考 [api.md](api.md) 的「消费的外部接口」与 [conventions.md](conventions.md) 的 CI `api-types` job。

## 补充说明

- schema 唯一定义在 `server/migrations/`（12 个迁移），权威接口定义在运行时 `/openapi.json`。
- 被删除的旧 `docs/plantree/`（原规划树）不在本归档内，其内容仅存于 git 历史；本归档是独立重建。
