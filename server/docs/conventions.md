# 约定

> 2026-08-30 依据源码与 CI 配置归纳。

## 代码风格

- rustfmt 全量（`cargo fmt --check` 是 CI 门禁）；clippy `-D warnings` 零容忍。
- 注释与文档以中文为主；doc comment 讲"为什么"，代码讲"做什么"。
- 错误统一 `ApiError`（stable code + retryable 标志），handler 返回 `Result<Json<T>, ApiError>`；
  错误体形状 `{"error":{"code","message","retryable"}}`。
- 长操作一律任务化：入 jobs 队列，不阻塞 HTTP 响应。
- 迁移只增不改：新变更新开 `NNNN_*.sql`，已发布迁移不回头编辑。

## 命名

- crate：领域名（distill/wiki-engine/…）；api crate 的路由文件 `<域>_api.rs`。
- 表：复数名词（sessions/atoms/…）；wiki_ 前缀区分 wiki 域。
- 环境变量：`AGENT_MEMORY_*` 前缀；测试专用 `AM_TEST_PG_URL`。

## 测试

- 集成测试为主（每 crate 一个 tests/ 目录），共享 `tests/support/mod.rs` 建一次性库。
- 100 用例全绿是合并前提；测试用 PG 由 CI 的 pgvector service 提供。

## Git 与提交

- 单 main 分支直推；Conventional Commits（`fix:`/`feat:`/`docs:`/`ci:`/`perf:`…，中文描述）。
- 提交按主题分块，一次关注点一个提交。

## CI（.github/workflows/）

| workflow | job | 内容 |
|---|---|---|
| ci.yml | backend | checkout 后 `mkdir -p ../web/dist`（rust-embed 编译期要求）→ fmt → clippy → test（pgvector/pgvector:pg17 service + AM_TEST_PG_URL） |
| ci.yml | web | pnpm install/lint/tsc/test/build |
| ci.yml | api-types | openapi-dump → openapi-typescript → 与 `web/src/lib/api-schema.ts` diff（零漂移门禁） |
| ci.yml | docker | 多阶段镜像构建验证 |
| e2e.yml | compose-e2e | compose 起全栈 + Playwright journey（无 provider 自动跳过 LLM 断言） |

注意：CI 后端用 stable 浮动工具链，本机过 clippy ≠ CI 过（1.97→1.98 曾新增 lint）；
推送前 `rustup update stable` 复验。
