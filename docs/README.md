# agent-memory 项目档案

> 全新初始化：2026-08-30，2026-09-01 全面更新（实体层/编辑分权/敏感清空体系/圈子页入档）。**三层结构**——本目录是全栈集成视角；
> 后端与前端各有独立项目档案。旧版文档已过期，本轮全部按当日源码/命令输出重写。

## 三层地图

| 层 | 入口 | 视角 |
|---|---|---|
| 全栈（本目录） | 本 README | 仓库整体、前后端接缝、跨栈约定 |
| 后端项目 | [server/docs/README.md](../server/docs/README.md) | Rust workspace 10 crates 的完整档案 |
| 前端项目 | [web/docs/README.md](../web/docs/README.md) | Engram SPA 的完整档案 |

产品事实与设计系统在仓库根：[PRODUCT.md](../PRODUCT.md)、[DESIGN.md](../DESIGN.md)。

## 本目录索引

| 文档 | 覆盖 | 何时读 |
|---|---|---|
| [overview.md](overview.md) | 产品定位、仓库形状、记忆蒸馏阶梯 | 30 秒了解全貌 |
| [architecture.md](architecture.md) | 系统总图、前后端接缝决策表 | 理解两端如何咬合 |
| [tech-stack.md](tech-stack.md) | 双端技术汇总（版本实查） | 选版本/排环境 |
| [api.md](api.md) | 67 端点域速览 + 分权矩阵 + 契约管理 | 找端点入口（全表在 server 档案） |
| [data-model.md](data-model.md) | 全栈数据流 + schema 三处同步点 | 改 schema 前 |
| [frontend-backend.md](frontend-backend.md) | 代理/托管/认证流//jobs 分流 | 两端集成问题 |
| [run-and-deploy.md](run-and-deploy.md) | 本地全栈最短路径 + 全量验证 | 跑起来 |
| [conventions.md](conventions.md) | 跨栈约定：提交/CI 门禁/同步点 | 协作前 |
| [current-state.md](current-state.md) | 当日验证矩阵、38 文件未提交批次、开放项 | **接手第一步** |

## 本目录其他内容

- [design/](design/audit.md)——前端设计审计证据（七维报告、双主题截图、色彩/度量 JSON）
- [plantree/](plantree/README.md)——规划树（frontend-polish 进行中；后端规划在 server/docs/plantree/）

## 快速上手（当日全绿命令）

```bash
cd server && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build
```
