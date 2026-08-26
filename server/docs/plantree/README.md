# Plan Tree

`agent-memory` 后端（`server/`）的规划树根。规划状态、决策、开放问题、证据归这里；**项目级基线由现有 `docs/` 承担**（`architecture.md` / `overview.md` / `data-model.md` / `tech-stack.md` / `conventions.md`），不重复建 baseline 文件。

## Active Plans

| Plan | Status | Current Phase | Last Landed | Next Target |
|---|---|---|---|---|
| [wiki-theory-integration](plans/wiki-theory-integration/README.md) | Done | status-update | theory.md + gap-analysis.md 已落 `docs/wiki/` | — |
| [backend-enhancement](plans/backend-enhancement/README.md) | In Progress | P0 Done（审计通过） | R1~R5 落地，66 tests passed | P1（R6~R12）待用户启动 |

## How to Read

1. 读目标 plan root 的 `README.md`（scope + authority + file map）
2. 读 `roadmap.md`（Done / In Progress / Next / Deferred）
3. 读 `topics/`（方案 + 初步发现）与 `open-questions.md`（未决）
