# P018 · 全量审查发现修复线（2026-10-07 七路）

> 来源：2026-10-07 code-review 技能全维度七路并发审查（errors/logging/deadcode/aifriendly/coupling/bugs/config），
> 范围 engram 当前 HEAD（含未提交改动），61 findings，两路抽查验真通过（P1 幂等键 SQL 逐行核实、
> AGENTS.md 迁移版本漂移 63↔75 核实）。
> 审查产物原始 JSON：[evidence/](evidence/)（七路 findings + summary 原文）；本树只追修复状态。

## Scope

**In**：bugs 路 P1×1 + P2×2 + P3×2 的修复；确定死代码删除；确证文档漂移修复；logging 级别纪律修正。
**Out**：需要拍板/较大重构的项（散点 env 集中化、guard.rs 七连复制拆合、DOMAIN_TOOLS 三方同步、
secrets 过滤机制、ATOM_MAX_CHARS 跨 crate 常量统一）——进 [open-questions.md](open-questions.md)；
errors 路传播链压平（JobError/LlmError→Unavailable 丢语义）归 P006 既有 Deferred 口径，本线不重复立项。

## Affected Modules

`server/jobs` `server/core` `server/wiki-engine` `server/api` `web` `scripts` `docs-repo(AGENTS.md/README/risk-hotspots)`

## 文件地图（阅读路径）

1. [roadmap.md](roadmap.md) — 任务状态（唯一任务权威）
2. [evidence/code-review-2026-10-07.md](evidence/code-review-2026-10-07.md) — 61 findings 全文（含「已查无问题」正面结论）
3. [open-questions.md](open-questions.md) — 等拍板的 5 项

## 执行纪律

- 门禁照旧：`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`；
  前端改动过 web 四门；MCP 面不动（本线无 MCP 改动）
- T001 修幂等键必须补回归用例（终态同键 enqueue 应发起新任务而非返回死任务）——现测试零覆盖
- 文档漂移条目（T007）修完顺手核对 engram projects 侧口径是否同步漂移（AGENTS.md 索引段声明「权威在服务端」）
- 修复后本 README/roadmap 记 commit 证据；对应 finding 在 evidence 标 ✅
