# Roadmap — agent-memory-platform

八阶段，每阶段一个领域垂直切片，出口即该领域生产质量（D0008）。

## 阶段总览

| Phase | 名称 | 依赖 | 状态 | 明细 |
|---|---|---|---|---|
| 0 | 项目地基 | — | **Done** (2026-10) | [phases/phase-0-foundation.md](phases/phase-0-foundation.md) |
| 1 | 核心底座 | 0 | **Next** | [phases/phase-1-core-infra.md](phases/phase-1-core-infra.md) |
| 2 | 记忆域（Chat Memory L0–L3） | 1 | Not started | [phases/phase-2-memory.md](phases/phase-2-memory.md) |
| 3 | 知识域（Knowledge） | 1 | Not started | [phases/phase-3-knowledge.md](phases/phase-3-knowledge.md) |
| 4 | Wiki 域 | 1 | Not started | [phases/phase-4-wiki.md](phases/phase-4-wiki.md) |
| 5 | CodeGraph 桥 | 1 | Not started | [phases/phase-5-codegraph.md](phases/phase-5-codegraph.md) |
| 6 | Web 控制台 | 2–5 | Not started | [phases/phase-6-web.md](phases/phase-6-web.md) |
| 7 | 交付打磨与发布 | 6 | Not started | [phases/phase-7-release.md](phases/phase-7-release.md) |

注：2/3/4/5 相互独立，可乱序或穿插执行；6 依赖各域 API 稳定。

## Done

- **Phase 0 项目地基**（2026-10）：9-crate workspace、Vite+React+shadcn 前端壳、
  多阶段 Docker + compose 全栈、CI 三 job、AGENTS 契约。
  出口门禁全绿（fmt/clippy/test/tsc/build/compose 探针 200），
  证据：[evidence/README.md](evidence/README.md) · 提交 `feat: phase-0 地基`

## In Progress

（无）

## Next

- Phase 1 核心底座（[phases/phase-1-core-infra.md](phases/phase-1-core-infra.md)）

## Deferred

（暂无）

## 硬约束（每阶段通用出口）

1. `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` 全绿
2. `tsc --noEmit` + eslint + vitest 全绿（涉及前端时）
3. 集成测试跑在 testcontainers 真 PG 上
4. docker-compose 起栈后该阶段功能可用
5. OpenAPI 快照与实现同步；文档与实现同步

详细门禁见 [baseline/test-and-release-gates](../../../baseline/test-and-release-gates.md)。
