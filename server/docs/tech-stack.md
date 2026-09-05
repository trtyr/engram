# 技术栈

> 2026-08-30 实查（Cargo.lock / Cargo.toml / 本机工具链）。版本以 lockfile 为准。

## 语言与工具链

| 项 | 值 | 出处 |
|---|---|---|
| Rust edition | 2024 | server/Cargo.toml |
| workspace version | 0.1.0 | server/Cargo.toml |
| 本机 rustc | 1.97.1（开发钉版） | rustc -V |
| CI 工具链 | stable 浮动（2026-08-30 为 1.98.0） | .github/workflows/ci.yml |
| 构建目标 | aarch64-apple-darwin（本地）/ linux（Docker） | — |
| release profile | thin LTO + strip | Cargo.toml [profile.release] |

## 核心 dependencies（Cargo.lock 实查）

| crate | 版本 | 用途 |
|---|---|---|
| axum | 0.8.9 | HTTP 框架（含 multipart 上传） |
| tokio | 1.53.1 | 异步运行时 |
| sqlx | 0.8.6 | PG 访问（migrate/uuid/chrono/json 宏） |
| pgvector | 0.4.2 | 向量列类型 |
| utoipa | 5.5.0 | OpenAPI 文档生成 |
| serde / serde_json | 1.0.229 | 序列化 |
| reqwest | 0.12.28（rustls） | LLM API 出站调用 |
| tracing / tracing-subscriber | 0.3.23 | 日志 |
| rust-embed | 8.12.0 | SPA 静态资源嵌入 |
| thiserror | 2.0.20 | 错误定义 |
| uuid / chrono | 1.24.1 / 0.4.45 | ID 与时间 |
| jieba-rs | 0.4.10 | 中文分词（检索融合） |
| pdf-extract | 0.12.0 | PDF 解析 |
| docx-rs | 0.4.22 | DOCX 解析 |
| scraper | 0.23.1 | HTML 清洗 |

## 存储

- PostgreSQL 16（本地 Homebrew 16.14；CI 用 pgvector/pgvector:pg17 service）
- 扩展：vector（迁移 0001 内 CREATE EXTENSION）

## 工具与质量门禁

| 工具 | 用途 | 命令 |
|---|---|---|
| rustfmt | 格式 | `cargo fmt --check` |
| clippy | 静态检查（-D warnings） | `cargo clippy --workspace --all-targets -- -D warnings` |
| cargo test | 单元+集成（189 用例，本机 PG） | `cargo test --workspace` |
| cargo audit | 依赖漏洞（1 接受项见 current-state） | `cargo audit` |
| cargo-chef | Docker 层缓存 | deploy/Dockerfile |
| openapi-dump | OpenAPI 导出 | `cargo run -q -p engram-api --bin openapi-dump` |

## 已接受的依赖风险（2026-08-30 cargo audit）

- rsa 0.9.10（RUSTSEC-2023-0071 Marvinattack，中危）：**lockfile 孤儿**，`cargo tree -i rsa --target all`
  零反向依赖，不参与任何编译产物——接受，不动 418 依赖的 lockfile。
- 4 条 transitive 提示（fxhash/rand_os/ttf-parser unmaintained、chacha20 yanked）：观察。
