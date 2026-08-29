# 技术栈

> 版本来源：`server/Cargo.toml` + `server/Cargo.lock`（2026-08-28 核实）、`web/package.json` + `web/pnpm-lock.yaml`。精确 patch 以 lockfile 为准。

## 后端（server/）

| 项 | 值 |
|---|---|
| 语言 | Rust，edition **2024** |
| 工具链 | rustc / cargo 1.97.x（本机 1.97.1；Dockerfile `cargo-chef:latest-rust-1.97-slim`） |
| 包管理 | Cargo workspace（`resolver = "2"`） |
| 构建 | `[profile.release]` `lto = "thin"` + `strip = true`；产物单二进制 `agent-memory-server` |
| 数据库 | PostgreSQL 17（镜像 `pgvector/pgvector:pg17`）+ 扩展 `vector` / `pg_trgm` |

核心库（Cargo.lock 精确版本）：axum 0.8.9（multipart）、tokio 1.53.1、tower/tower-http 0.5/0.6（trace/cors/fs）、sqlx 0.8.6（postgres/macros/migrate/uuid/chrono/json）、serde 1.0.229、utoipa 5.5.0、uuid 1.24.1（v7）、chrono 0.4.45、thiserror 2.0.20、tracing/tracing-subscriber 0.1.44/0.3.23（JSON 日志）。

领域库：pgvector 0.4.2、jieba-rs 0.4.10（中文分词）、reqwest 0.12.28（rustls）、aes-gcm/sha2/rand（密钥 AES-256-GCM）、pdf-extract 0.12.0（lopdf ≥0.42，RUSTSEC-2026-0187 已修）、docx-rs 0.4.22、scraper 0.23.1、rust-embed 8.12.0（内嵌 SPA）。

外部依赖：LLM provider（OpenAI 兼容 HTTP）、codegraph CLI（`@colbymchenry/codegraph@1.5.0`，子进程 `--json`）。

## 前端（web/）

| 项 | 值 |
|---|---|
| 语言 | TypeScript ~6.0（锁定 6.0.3） |
| 包管理 | **pnpm**（pnpm-lock.yaml + packageManager 字段钉 11.20.0；`.npmrc` = `legacy-peer-deps=true`；CI/Dockerfile 已同步 corepack/pnpm） |
| UI 框架 | React **19.2.8** + react-dom |
| 构建 | Vite **8.2.2**（rolldown；`tsc -b && vite build`） |
| 路由 | react-router-dom 7.18.2 |
| 服务端状态 | @tanstack/react-query 5.102.3 |
| 样式 | Tailwind CSS **4.3.3**（`@tailwindcss/vite` 插件）+ tw-animate-css 1.4.0 |
| UI 件 | radix-ui 1.6.7（shadcn 风格，`components.json`）、lucide-react 1.34.0、cva + tailwind-merge |
| 可视化 | sigma 3.0.3 + graphology 0.26（Wiki 图谱）、mermaid 11.17.2、react-markdown 10 |
| 字体 | @fontsource-variable/geist |

质量工具：oxlint 1.80.0（lint）、vitest 4.1.11 + jsdom + @testing-library（单测/组件测）、@playwright/test 1.62.1（e2e）、openapi-typescript 7.13.0（类型生成）、lighthouse 13.4.1（devDependency，性能预算用）。

## 工具链命令对照

| 用途 | 后端（在 server/） | 前端（在 web/） |
|---|---|---|
| 安装 | —（Cargo） | `pnpm install --frozen-lockfile` |
| 构建 | `cargo build` | `pnpm run build`（tsc -b + vite build） |
| 格式/lint | `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` | `pnpm run lint`（oxlint） |
| 测试 | `cargo test --workspace`（需本机 PG，见下） | `pnpm test`（vitest） / `npx playwright test`（e2e，需运行中的栈） |
| 类型检查 | clippy 内含 | `tsc -b`（build 的一部分） |

后端集成测试基建：本机 PostgreSQL（Homebrew，trust 认证），每测试建/删独立库 `am_test_*`；`AM_TEST_PG_URL` 可覆盖管理库连接串（各 crate `tests/support/mod.rs`）。CI 的 backend job 已配 `pgvector/pgvector:pg17` service + `AM_TEST_PG_URL`。

## CI/CD

GitHub Actions（`.github/workflows/`）：

- `ci.yml`（push/PR）：`backend`（fmt → clippy → test，pgvector service）、`web`（pnpm install → lint → tsc → vitest → build）、`api-types`（openapi-dump → 生成 → 漂移检查）、`docker`（构建 deploy/Dockerfile 镜像）。
- `e2e.yml`（push main/PR）：compose 栈起全链 → Playwright 全旅程（`E2E_BASE`/`E2E_ADMIN_PW`；无 provider 时 LLM 断言自动跳过）。
