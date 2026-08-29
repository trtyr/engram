# 约定

> 后端约定详版见 [server/docs/conventions.md](../server/docs/conventions.md)（仍有效，摘要如下）；本文档补全仓视角（前端约定 + git + CI）。

## 后端（server/）摘要

- Rust edition 2024，rustfmt 默认配置门禁；每 crate/关键文件顶部 `//!` 职责说明。
- 错误：`thiserror` 类型化错误 → API 边界收敛 `ApiError`，错误体 `{"error":{code,message,retryable}}`；内部细节只进 tracing 日志。
- 日志：`tracing` 结构化，禁止 `println!`；生产 JSON，`RUST_LOG` 控级别。
- sqlx：运行时 API（`query`/`query_as`），不用编译期宏；schema 只能新增迁移文件，不改历史迁移。
- 提示词：版本化 `PromptId(name, version)`；改模板必须升版本（产物记 `prompt_version` 可回放）。
- 模块边界：api 只经 core；storage 是薄层；叶子 crate（llm/jobs/search/cg-bridge/parsing）不依赖内部；禁止域间横向 import。
- 通用：ISO8601 UTC、uuid v7、embedding 1024 维、tsv 应用层 jieba 预分词。

## 前端（web/）

- **包管理是 pnpm**（2026-08-28 迁移，`packageManager: pnpm@11.20.0` 钉版本）：`pnpm install --frozen-lockfile`；CI 用 pnpm/action-setup，Dockerfile 用 corepack。
- 目录：`features/` 每域一页（与后端路由组一一对应）、`components/` 跨页复用件、`lib/` 基础设施（api client / 生成类型 / 工具）；`@/` 别名指向 `src/`。
- 测试共置：组件测试 `*.test.tsx` 与源文件同目录（vitest + jsdom + @testing-library）；e2e 独立 `e2e/` 目录归 Playwright（vitest exclude 排除之）。
- 数据获取：统一走 `lib/api.ts` 的封装（Bearer 注入 + 401 清 token + `ApiError`），服务端状态用 react-query；不直接散写 fetch。
- 样式：Tailwind 4 语义 token（border/card/muted/brand 等），UI 件 radix-ui（shadcn 风格，`components.json`）；图标 lucide-react。
- lint：oxlint（`.oxlintrc.json` 可选 type-aware）；类型检查 `tsc -b` 内含于 build。

## Git 工作流

- 主分支 `main`，直接推送；CI 在 push 与 PR 都触发。
- Conventional Commits 带作用域：`fix(web): ...`、`feat(wiki): ...`、`docs(plantree): ...`、`refactor(llm): ...`。
- 版本：workspace 0.1.0（`server/Cargo.toml` `[workspace.package]`）；web package.json 0.0.0（不独立发版）。
- 历史分工（git log/plantree 记载）：后端 agent 只负责 `server/`，`web/` 由另一人（agent）负责，两边经 OpenAPI 契约对齐（`server/docs/plantree/plans/` 有 50 调用点契约核对记录）。

## CI 过程

`.github/workflows/ci.yml`（push/PR）：

| job | 内容 | 当前状态 |
|---|---|---|
| `backend` | fmt → clippy(-D warnings) → test（pgvector/pg17 service + AM_TEST_PG_URL） | ✅ 本地三项全绿（100 tests） |
| `web` | `pnpm install --frozen-lockfile` → lint → tsc → vitest → build | ✅ 本地全绿（21 tests，无大 chunk 警告） |
| `api-types` | openapi-dump → 生成 → git diff 漂移检查 | ✅ 零漂移 |
| `docker` | 构建 deploy/Dockerfile 镜像 | web-build 阶段本地验证 ✅；完整构建由 CI 验证 |

`.github/workflows/e2e.yml`（push main/PR）：compose 栈 + Playwright 全旅程；无 LLM provider 时蒸馏/Wiki 生成断言自动跳过（annotation 留痕）——本地无 provider 栈实测 PASS。
