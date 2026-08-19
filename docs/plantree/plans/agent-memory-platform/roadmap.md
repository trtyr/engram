# Roadmap — agent-memory-platform

八阶段，每阶段一个领域垂直切片，出口即该领域生产质量（D0008）。

## 阶段总览

| Phase | 名称 | 依赖 | 状态 | 明细 |
|---|---|---|---|---|
| 0 | 项目地基 | — | **Done** (2026-10) | [phases/phase-0-foundation.md](phases/phase-0-foundation.md) |
| 1 | 核心底座 | 0 | **Done** (2026-10) | [phases/phase-1-core-infra.md](phases/phase-1-core-infra.md) |
| 2 | 记忆域（Chat Memory L0–L3） | 1 | **Done** (2026-10) | [phases/phase-2-memory.md](phases/phase-2-memory.md) |
| 3 | 知识域（Knowledge） | 1 | **Done** (2026-10) | [phases/phase-3-knowledge.md](phases/phase-3-knowledge.md) |
| 4 | Wiki 域 | 1 | **Next** | [phases/phase-4-wiki.md](phases/phase-4-wiki.md) |
| 5 | CodeGraph 桥 | 1 | Not started | [phases/phase-5-codegraph.md](phases/phase-5-codegraph.md) |
| 6 | Web 控制台 | 2–5 | Not started | [phases/phase-6-web.md](phases/phase-6-web.md) |
| 7 | 交付打磨与发布 | 6 | Not started | [phases/phase-7-release.md](phases/phase-7-release.md) |

注：2/3/4/5 相互独立，可乱序或穿插执行；6 依赖各域 API 稳定。

## Done

- **Phase 3 知识域**（2026-10）：摄取管道三步链（parse→chunk→embed）+ SSRF 安全抓取
  （DNS pinning/逐跳复检/私网全拒/20MB 30s）+ 四格式解析（md/pdf/docx/html）+
  sha256 幂等 + 嵌入降级 FTS + /knowledge/* 7 端点。真 e2e 三类文档 ready +
  中文命中；修 3 个真 bug，见 [evidence/README.md](evidence/README.md)
- **Phase 2 记忆域**（2026-10）：search（jieba 预分词 + pgvector + RRF 融合）+
  distill（extract/arbitrate/organize/persona/consolidate 五阶段链 + 版本化提示词 +
  LLM I/O 全量记入 job_events）+ core MemoryService（L0 写入/防抖触发/检索/context 包/
  atoms 治理/画像历史回滚）+ /memory/* 12 端点。
  真 LLM e2e 全断言通过（矛盾 supersede + 画像 v1→v2 + 三层 context 引用链），
  途中修 5 个真 bug，见 [evidence/README.md](evidence/README.md)
- **Phase 1 核心底座**（2026-10）：10 份迁移（16 表全域 schema 定版）+ jobs 任务系统
  （抢占/退避重试/僵尸回收/幂等/事件/Runner）+ llm 出口（provider 抽象/AES-GCM 密钥加密/
  purpose 路由/用量记账）+ 鉴权（opaque 会话 + API key scopes）+ OpenAPI 快照。
  出口门禁全绿含真实网关连通证据，见 [evidence/README.md](evidence/README.md)
- **Phase 0 项目地基**（2026-10）：9-crate workspace、Vite+React+shadcn 前端壳、
  多阶段 Docker + compose 全栈、CI 三 job、AGENTS 契约。
  出口门禁全绿（fmt/clippy/test/tsc/build/compose 探针 200），
  证据：[evidence/README.md](evidence/README.md) · 提交 `feat: phase-0 地基`

## In Progress

（无）

## Next

- Phase 4 Wiki 域（[phases/phase-4-wiki.md](phases/phase-4-wiki.md)）

## Deferred

（暂无）

## 硬约束（每阶段通用出口）

1. `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` 全绿
2. `tsc --noEmit` + eslint + vitest 全绿（涉及前端时）
3. 集成测试跑在 testcontainers 真 PG 上
4. docker-compose 起栈后该阶段功能可用
5. OpenAPI 快照与实现同步；文档与实现同步

详细门禁见 [baseline/test-and-release-gates](../../../baseline/test-and-release-gates.md)。
