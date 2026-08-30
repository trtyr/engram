# 约定（全栈）

> 细则分栈：[server/docs/conventions.md](../server/docs/conventions.md)、
> [web/docs/conventions.md](../web/docs/conventions.md)。本文是跨栈共识。

## Git 与提交

- 单 main 分支直推；Conventional Commits（中文描述），按主题分块提交。
- 推送后用 gh 盯 CI + e2e 双 workflow 到绿才算完（既定工作流）。

## CI 门禁（全绿才可合）

| 门 | 内容 |
|---|---|
| backend | fmt → clippy(-D warnings) → test(100)；pgvector service；`mkdir -p ../web/dist` 占位 |
| web | oxlint（0 警告）→ tsc → vitest(26) → build |
| api-types | OpenAPI 导出 → 生成 → 与 api-schema.ts 零漂移 |
| docker | 多阶段镜像构建 |
| e2e | compose 全栈 + Playwright journey（无 provider 部分旅程） |

## 跨栈同步点（改一端想另一端）

| 改动 | 必须同步 |
|---|---|
| 后端端点/DTO | `pnpm run gen:api` 重生成 + web/src/lib/api.ts 手写类型核对 + e2e |
| Cargo.toml version | 无需动前端（vite define 自动读） |
| 前端路由 | 检查是否与后端 API 路径碰撞（先例：/jobs） |
| 表单控件/按钮文案 | e2e 选择器（getByLabel / getByRole 中文 name，exact 防撞） |
| web/dist 相关 | 后端 rust-embed 编译期要求目录存在（CI 已占位） |

## 工具链纪律

- CI 后端 stable 浮动：推送前 `rustup update stable` 本地复验 clippy。
- web lint 必须写全 `pnpm run lint`（裸 `pnpm lint` 有误导性报错）。
- 锁文件进库（Cargo.lock + pnpm-lock.yaml），`--frozen-lockfile` 安装。

## 文档结构（本仓库三层）

根 docs/（集成）→ server/docs/ 与 web/docs/（分栈档案）→ 各自 plantree/（活规划）。
产品与设计事实在 PRODUCT.md / DESIGN.md（根）。改架构时同步对应层档案。
