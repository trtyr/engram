# Implementation Status — agent-memory-platform

> 仅 `In Progress` 阶段的操作交接。权威路线见 [roadmap.md](roadmap.md)。

## Current Phase

（Phase 0 已完成，待领 Phase 1）

## Last Landed

- 2026-10 `b79e67f` Phase 0 项目地基：9-crate workspace + 前端壳 + compose 全栈 + CI。
  出口门禁全绿，证据 [evidence/README.md](evidence/README.md)

## Active TODO

（进入 Phase 1 时更新：全量 schema 迁移 → jobs crate → llm crate → 鉴权 → OpenAPI 快照）

## Blocked By

（无）

## Last Verified

- 2026-10（Phase 0 出口）：fmt/clippy(-D warnings)/test 全绿；web tsc+build 绿；
  compose 栈 `/health`→`{"status":"ok"}`、`/ready`→`{"status":"ready","migration_version":1}`、
  `/openapi.json` 正常、404 统一错误体。本地端口 19180（8080/18080 被其他项目占用）
- deploy/.env 为本地验证临时文件（gitignored），含测试密钥，勿提交

## Phase 0 交付物清单（对照 phases/phase-0-foundation.md）

- [x] git init + .gitignore + .dockerignore
- [x] server/ workspace：9 crates（api 完整骨架，storage 真实现，其余 7 个空壳立边界）
- [x] web/：Vite + React 19 + TS + Tailwind v4 + shadcn/ui（radix preset）+ 路由壳（七域导航）
- [x] deploy/：多阶段 Dockerfile（cargo-chef 分层缓存）+ compose（pgvector + app + healthcheck）+ .env.example
- [x] CI：backend（fmt/clippy/test）+ web（lint/tsc/build）+ docker build 三 job
- [x] AGENTS.md + README.md
- [ ] compose 起栈端到端验证（进行中）
