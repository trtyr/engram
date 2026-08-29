# 约定

> 来源：`server/` 代码实际做法 + CI（`.github/workflows/`）。前端约定（web/）不在本文档范围。

## 语言与风格

- **Rust edition 2024**；`rustfmt` 默认配置（`cargo fmt --check` 门禁）。
- 模块文档：每个 crate/lib 顶部有 `//!` 说明职责与依赖方向；关键文件顶部 `//!` 说明本文件职责。
- 依赖方向以各 crate `Cargo.toml` 的 `[dependencies]` 为准（见 [architecture.md](architecture.md)）。

## 错误处理

- 用 `thiserror` 定义类型化错误（各 crate 有自己的错误枚举，如 `MemoryError`、`LlmError`、`JobError`、`WikiError`、`ParseError`）。
- API 边界统一收敛到 `ApiError`（`server/crates/api/src/error.rs`），错误体契约 `{"error":{code,message,retryable}}`。
- `code` 稳定可编程判断；`message` 人话且不泄漏内部细节；内部细节（Debug 含 source 链）只进 `tracing::error!` 日志，一次性记录。

## 日志

- 用 `tracing`（结构化字段），**禁止 `println!`**。
- 生产日志 JSON 格式（`tracing_subscriber::fmt().json()`），级别由 `RUST_LOG` 控制（默认 `info`）。

## 数据访问（sqlx）

- **运行时 API**（`query` / `query_as` / `QueryBuilder`），不用编译期 `query!` 宏——避免构建依赖 `DATABASE_URL`。
- schema 只能改 `server/migrations/`，**新增迁移文件，不改已应用的历史迁移**；启动时自动执行。

## 提示词模板

- 版本化 `PromptId(name, version)` 常量（`distill` / `wiki-engine` 的 `prompts.rs`）。
- 修改任何模板**必须升版本号**——蒸馏产物记录 `prompt_version`，可归因可回放。

## 通用约定

- 时间：ISO8601 UTC（`timestamptz`）。
- ID：uuid **v7**（`Uuid::now_v7()`）。
- embedding 统一 1024 维（bge-m3）；全文检索 `tsv` 由应用层 jieba 预分词维护，写入与查询同源。

## 模块边界规则

- 域逻辑统一放 `core`；api 只经 `core` 访问域服务（wiki/codegraph 是 core 里的门面 re-export）。
- `storage` 是 pool + 迁移薄层；查询直接用 sqlx 写在 `core`/`api`（未建仓储抽象层）。
- `llm` / `jobs` / `parsing` 不依赖任何内部 crate（叶子层）。
- **禁止域 crate 之间横向 import**（如 `distill` → `wiki-engine`）；跨域编排放 `core`。

## 鉴权

- 管理员会话 token `ams_` 前缀（opaque，sha256 落库，7 天过期，恒定时间比较）。
- API key `amk_` 前缀（sha256 落库 + 明文只返回一次，scopes 限定）。
- scope 值：`memory` / `knowledge` / `wiki` / `codegraph`。

## Git 工作流

- 主分支 `main`，直接推送（CI 在 push 与 PR 都触发）。
- commit 消息用 Conventional Commits 风格，带作用域，如 `fix(e2e): ...`、`docs: ...`、`refactor(Q8-Q10 技术债清理): ...`、`fix(ci): ...`。
- 发布版本：workspace 版本 `0.1.0`（`[workspace.package]`）。

## CI 过程（`.github/workflows/ci.yml`）

| job | 内容 |
|---|---|
| `backend` | `cargo fmt --check` → `cargo clippy --workspace --all-targets -- -D warnings` → `cargo test --workspace`（rust-cache；pgvector/pgvector:pg17 service + `AM_TEST_PG_URL` 供集成测试） |
| `web` | `pnpm install --frozen-lockfile` → lint → tsc → vitest → build（前端，不在本文档范围） |
| `api-types` | `openapi-dump` 提取 → `openapi-typescript` 再生成 → git diff 漂移检查（后端 API 变更必须同步前端类型） |
| `docker` | 构建 `deploy/Dockerfile` 镜像 |

另 `.github/workflows/e2e.yml`：起 compose 栈跑 Playwright 全旅程（构建 web + openapi → compose up → playwright test → 失败 dump 日志）。

## 质量门禁清单

- fmt / clippy / test 全绿；compose 可起；文档同步；证据留档。
- 后端 API 变更后必须重跑 openapi-dump 流程（否则 CI `api-types` job 红）。
