# Phase 0 — 项目地基

**目标**：可启动、可 CI、可协作的空平台。所有骨架一次到位，后续阶段只往里填肉，不再动结构。

## 前置

- 无（起点）

## 交付物

### 仓库与工程

- [ ] git init + `.gitignore`（Rust + Node + data/ + .env）
- [ ] `server/`：cargo workspace，`crates/{api,core,storage,llm,jobs,distill,wiki-engine,cg-bridge,search}` 空壳（lib.rs 各自一行 doc comment，依赖关系按 module-map 约束声明）
- [ ] `web/`：Vite + React 19 + TS 脚手架，Tailwind + shadcn/ui 初始化，目录按 module-map（features/components/lib/stores）
- [ ] `deploy/`：`Dockerfile`（多阶段骨架）、`docker-compose.yml`（app + pgvector/pgvector:pg17 + 卷 + healthcheck）、`.env.example`
- [ ] `AGENTS.md`：仓库操作契约（构建/测试命令、模块边界、编码规范要点）
- [ ] 根 `README.md`：项目一句话 + 快速启动（内容随阶段充实）

### 后端骨架

- [ ] api crate：axum 启动、`GET /health` `GET /ready`、tracing JSON 日志中间件、统一错误体类型（空实现先立契约）
- [ ] storage crate：sqlx PgPool 装配 + 迁移 runner（`sqlx migrate run` 启动时执行）、testcontainers 测试基建
- [ ] 配置：env 加载（figment/config crate），含 DATABASE_URL、MASTER_KEY、ADMIN_PASSWORD

### CI

- [ ] GitHub Actions：fmt + clippy + test（带 PG service 或 testcontainers）、前端 lint+tsc+build、docker build 验证——全套真实跑通，哪怕测试只有骨架断言

## 出口标准

1. `docker compose up -d` → pg + app 健康，`/health` `/ready` 200
2. CI 全绿（含 docker build）
3. `cargo test` 内含至少一个 testcontainers 集成测试样例（验证基建）
4. workspace 依赖方向静态可查（clippy workspace lints 或 cargo-deny 检查无环）

## 关联

- 模块边界：[baseline/module-map](../../../baseline/module-map.md)
- 决策：D0006
