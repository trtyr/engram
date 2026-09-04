# Engram server — 项目档案

> 后端 Rust workspace 的完整书面档案。2026-08-30 全新初始化（此前版本已过期，全部按当日源码与命令输出重写）。
> 上层集成视角见[仓库根 docs/](../../docs/README.md)；前端项目档案见 [web/docs/](../../web/docs/README.md)。

## 这是什么

单用户 AI 长期记忆服务的后端：记忆蒸馏（L0 会话→L1 原子→L2 场景→L3 画像）、知识库（文档→分块→向量）、
LLM Wiki（摄取→分析→生成→图谱）、CodeGraph（代码库索引）、任务队列与 LLM 网关。
HTTP API（axum）+ PostgreSQL(pgvector)，SPA 静态资源经 rust-embed 同源托管。

## 档案索引

| 文档 | 覆盖 | 何时读 |
|---|---|---|
| [overview.md](overview.md) | 产品定位、核心能力、整体形状 | 30 秒了解这是什么 |
| [architecture.md](architecture.md) | 10 crate 地图、模块职责、依赖方向 | 找代码从这开始 |
| [tech-stack.md](tech-stack.md) | 语言/框架/关键依赖版本（lockfile 实查） | 排查版本问题 |
| [api.md](api.md) | 全部 88 路径 / 113 方法注册（当日 OpenAPI 活体导出）+ 分权矩阵 | 对接前端/写客户端 |
| [data-model.md](data-model.md) | 27 张业务表、26 个迁移、数据流 | 改 schema 前必读 |
| [run-and-deploy.md](run-and-deploy.md) | 本地起栈、测试、环境变量、部署 | 跑起来 |
| [conventions.md](conventions.md) | 代码风格、错误处理、测试、git/CI 约定 | 写代码前 |
| [current-state.md](current-state.md) | 当日验证基线（命令+退出码）、未提交变更、开放项 | 接手第一步 |

## 补充资料（非本档案核心，保留的历史深潜）

- [plantree/](plantree/README.md)——后端规划树（活文档）
- [wiki/](wiki/README.md)——LLM Wiki 子系统设计文档（theory/ingest/graph/api）
- [memory-audit.md](memory-audit.md) / [knowledge-audit.md](knowledge-audit.md) / [llm-audit.md](llm-audit.md) / [wiki-audit.md](wiki-audit.md)——各域历史审计

## 快速验证（2026-09-03 实跑）

```bash
cd server
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings   # 均 exit 0
cargo test --workspace        # 176 passed / 0 failed（36 个套件）
cargo run -q -p engram-api --bin openapi-dump   # OpenAPI 导出：88 路径 / 113 方法
```
