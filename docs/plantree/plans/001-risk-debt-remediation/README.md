# P001 · 风险债整改线

> 目标：把 init-project 全量接入挖出的 22 项风险债按优先级消化掉。
> 风险详情权威：档案「风险与债」01a0e6f6-04d2-7b03-b128-29863af0e008（本树只追整改状态）。

## Scope

**In**：22 项风险的修复/收敛/拍板流转；敏感语义统一（决策 001）的方案与落地。
**Out**：新功能开发；性能重架构（10k 页优化单独立 plan，本线只记录）；文档再重写。

## Affected Modules

`scripts` `deploy` `docs-repo` `server/distill` `server/core` `server/mcp` `web`（逐任务见 roadmap）

## 文件地图（阅读路径）

1. [roadmap.md](roadmap.md) — 任务状态（唯一任务权威）
2. [topics/risk-inventory.md](topics/risk-inventory.md) — 22 项风险→处置映射
3. [decisions/001-sensitive-as-pure-marker.md](decisions/001-sensitive-as-pure-marker.md) — 已拍板：敏感降级纯标记
4. [open-questions.md](open-questions.md) — **等你拍板的（当前 1 个：Q004）**
5. [implementation-status.md](implementation-status.md) — 执行交接现状（T001-T004 已提交 9be3bec..bde6092）

## 执行纪律

- 纯 bug/文档对齐类（T001-T003）无需拍板，直接修
- 触及语义/口径的任务（敏感放开、读权限不对称）必须先有 decision 才动代码
- 每个任务落地后在本 README Last Landed 补 commit/验证证据
- 改检索必须跑 wiki-benchmark；改 distill 必须 cargo test --workspace（见 baseline/test-and-release-gates.md）
