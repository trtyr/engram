# Engram web — 项目档案

> 前端 SPA 的完整书面档案。2026-08-30 全新初始化（旧文档只有一份对接审计，已不足以描述现状）。
> 上层集成视角见[仓库根 docs/](../../docs/README.md)；后端档案见 [server/docs/](../../server/docs/README.md)。

## 这是什么

**Engram**（神经科学：记忆痕迹）——单用户 AI 长期记忆平台的控制台前端。
七页信息架构（概览 / 用户记忆 / 圈子 / Wiki / 代码图谱 / 任务 / 设置），
墨白正统设计语言（Vercel/Geist 谱系，双主题）。React 19 + TS + Tailwind 4 + Vite 8 + pnpm。

设计系统的权威文档在仓库根 [DESIGN.md](../../DESIGN.md)（视觉世界、token、禁区清单）与
[PRODUCT.md](../../PRODUCT.md)（产品事实）；本档案聚焦工程面。

## 档案索引

| 文档 | 覆盖 | 何时读 |
|---|---|---|
| [overview.md](overview.md) | 页面与能力地图、设计世界一句话 | 30 秒了解 |
| [architecture.md](architecture.md) | src 结构、组件分层、数据流 | 找代码从这开始 |
| [tech-stack.md](tech-stack.md) | 依赖版本（lockfile 实查）、构建链 | 版本问题 |
| [api.md](api.md) | 前端视角的 API 面：调用约定、类型生成、认证 | 对接后端 |
| [data-model.md](data-model.md) | 前端域类型与状态模型 | 数据形状 |
| [run-and-deploy.md](run-and-deploy.md) | dev/build/test/e2e 命令、代理、嵌入产物 | 跑起来 |
| [conventions.md](conventions.md) | 设计纪律、代码约定、测试策略 | 写代码前 |
| [current-state.md](current-state.md) | 当日验证基线、bundle 体积、开放项 | 接手第一步 |
| [api-alignment-audit.md](api-alignment-audit.md) | 历史对接审计（2026-08-28，44 调用点零缺漏） | 追溯参考 |

## 快速验证（2026-08-30 实跑）

```bash
cd web
pnpm install --frozen-lockfile
pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build
# 全部 exit 0；lint 0 警告；vitest 26/26
```
