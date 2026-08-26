# 技术栈

> 版本来源：`server/Cargo.toml`（workspace 约束）与 `server/Cargo.lock`（实际解析的精确版本）。精确 patch 版本以 Cargo.lock 为准。

## 语言与工具链

| 项 | 值 |
|---|---|
| 语言 | Rust，edition **2024** |
| 工具链 | rustc / cargo 1.97.x（本机 1.97.1；Dockerfile 用 `rust:1.97-slim`） |
| 包管理 | Cargo（workspace，`resolver = "2"`） |
| 构建配置 | `[profile.release]` `lto = "thin"` + `strip = true` |
| 许可证 | MIT |
| 仓库 | github.com/trtyr/agent-memory |

## 核心框架与库（精确版本）

| 库 | 版本 | 用途 |
|---|---|---|
| tokio | 1.53.1 | 异步运行时（`features = ["full"]`） |
| axum | 0.8.9 | HTTP 框架（`multipart` feature） |
| tower / tower-http | 0.5.3 / 0.6.11 | 中间件（trace/cors/fs） |
| sqlx | 0.8.6 | Postgres 异步驱动（`postgres`, `macros`, `migrate`, `uuid`, `chrono`, `json`, `runtime-tokio-rustls`） |
| serde / serde_json | 1.0.229 / 1.0.151 | 序列化 |
| utoipa | 5.5.0 | OpenAPI 文档（`axum_extras`, `uuid`, `chrono`） |
| uuid | 1.24.1 | ID（v7/v4，`serde`） |
| chrono | 0.4.45 | 时间 |
| thiserror / anyhow | 2.0.20 / 1.0.104 | 错误处理 |
| tracing / tracing-subscriber | 0.1.44 / 0.3.23 | 结构化日志（`env-filter`, `json`） |

## 领域库

| 库 | 版本 | 用途 |
|---|---|---|
| pgvector | 0.4.2 | 向量类型 + HNSW 索引（`sqlx` feature） |
| jieba-rs | 0.4.10 | 中文分词（应用层预分词 → tsvector） |
| reqwest | 0.12.28 | LLM HTTP 客户端（`json`, `rustls-tls`） |
| aes-gcm / sha2 / rand | 0.10.3 / 0.10.9 / 0.9.x | 密钥加密（AES-256-GCM）、哈希（登录/API key）、随机 |
| pdf-extract | 0.12.0 | PDF → 文本（内部 lopdf ≥0.42，RUSTSEC-2026-0187 已修复） |
| docx-rs | 0.4.22 | DOCX → 文本 |
| scraper | 0.23.1 | HTML → 文本 |
| rust-embed | 8.12.0 | 内嵌 `web/dist` SPA 静态资源 |
| mime_guess | 2.0.5 | 上传 Content-Type 推断 |

## 测试工具

| 库 | 版本 | 用途 |
|---|---|---|
| testcontainers | 0.24.0 | 集成测试拉起真实 PostgreSQL 容器 |
| tempfile | 3 | 测试临时目录 |

## 数据库

- **PostgreSQL 17**（生产镜像 `pgvector/pgvector:pg17`）
- 扩展：`vector`（pgvector）、`pg_trgm`
- schema 由 `server/migrations/` 12 个迁移文件定义，启动时由 `sqlx` 自动执行。

## 质量门禁工具

| 工具 | 命令 | 说明 |
|---|---|---|
| rustfmt | `cargo fmt --check` | 格式检查 |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 警告即错误 |
| cargo test | `cargo test --workspace` | 单元 + 集成（testcontainers 真 PG） |
| cargo audit | `cargo audit` | 依赖漏洞基线 |

CI 门禁见 [conventions.md](conventions.md)，运行命令见 [run-and-deploy.md](run-and-deploy.md)。
