# 技术栈（全栈汇总）

> 版本数字以两端 lockfile 实查为准（2026-08-30 实查，2026-09-01 复核无依赖升级）。明细：
> [server/docs/tech-stack.md](../server/docs/tech-stack.md)、[web/docs/tech-stack.md](../web/docs/tech-stack.md)。

| 层 | 技术 | 版本 |
|---|---|---|
| 后端语言 | Rust（edition 2024，workspace 0.1.0） | 本机 1.97.1 / CI stable(1.98) |
| 后端框架 | axum + tokio + sqlx | 0.8.9 / 1.53.1 / 0.8.6 |
| API 文档 | utoipa（OpenAPI 导出+前端类型生成） | 5.5.0 |
| 存储 | PostgreSQL + pgvector（本地 16.14 / CI pgvector:pg17） | — |
| 前端语言 | TypeScript | 6.0.3 |
| 前端框架 | React + react-router-dom | 19.2.8 / 7.18.2 |
| 样式 | Tailwind CSS（CSS-first token） | 4.3.3 |
| 构建 | Vite（+tsc -b） | 8.2.2 |
| 包管理 | pnpm（web）；cargo（server） | 11.20.0 |
| 测试 | cargo test（100）+ vitest（26）+ Playwright（journey） | — |
| Lint | clippy -D warnings + oxlint | — |
| 字体 | Geist / Geist Mono（@fontsource-variable 5.3.0） | — |
| 重可视化 | mermaid 11.17.2 / sigma 3.0.3 / cytoscape（懒加载） | — |
| 部署 | Docker 多阶段（node:22-slim + cargo-chef rust-1.97） | 开发期未启用 |

## 跨栈共享物

- **OpenAPI 契约**：server 导出 → web 生成 api-schema.ts（CI 零漂移）。
- **版本号**：server/Cargo.toml workspace version → vite define → 前端 `v{__APP_VERSION__}`。
- **web/dist**：前端构建产物由后端 rust-embed 编进二进制（编译期要求目录存在）。
