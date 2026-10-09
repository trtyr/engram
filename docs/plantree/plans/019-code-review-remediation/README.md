# P019 · code-review 整改线（2026-10-09 全量审查）

## 范围

2026-10-09 code-review 技能全量审查（30 微观功能点 + 8 宏观维度）产出的排查整改。
报告入口：[docs/review/index.md](../../../review/index.md)（36 篇检查项文档 + `.chain.json` 链路底稿）。

**排查结论**：主 agent 已逐篇通读全部检查项，6 条关键发现回源码核实（5 确证 / 1 误报）。
已确证误报：search-crate 报「search_chunks vec CTE 漏 LIMIT」——实际
`storage/repo/wiki_docs.rs:395` 有 `LIMIT 200`，**不立项**。

- Affected Modules：server/jobs, server/cg-bridge, server/distill, server/wiki-engine, server/api, server/mcp, server/core, server/storage, web, scripts
- 门禁：server `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`；web 四件套（见 baseline/test-and-release-gates）
- 分工：Mia 整理并执行；trtyr 拍板

## 文件

- [roadmap.md](roadmap.md) — 任务清单（唯一任务权威）
- [open-questions.md](open-questions.md) — 待拍板点
