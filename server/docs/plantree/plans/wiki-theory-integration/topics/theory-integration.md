# Topic: 理论整合方案

把「为什么这么设计」这一层补进 `docs/wiki/`。现文档是纯实现视角，缺理论来源与设计意图。

## 新增 docs/wiki/theory.md

结构（一份文档，不分碎）：

1. **理论谱系**：Karpathy 原文 → 两个落地分支（TencentDB 团队级 / nashsu 个人级）→ 本项目的定位（团队级、Rust 重写 TencentDB 思路）。
2. **三层架构**：raw sources（不可变）/ wiki（LLM 全权）/ schema+purpose（规范与方向）——对应本实现的 `wiki_sources` / `wiki_pages` / `prompts.rs`+`purpose.rs`。
3. **三操作**：ingest / query / lint 的设计意图，各自对应到实现文件。
4. **index.md / log.md** 的设计意图（内容目录 vs 时间线），对应实现的 `index` / `log` / `overview` 系统页 + `job_events`。
5. **理论 → 实现映射表**：一列列理论概念，一列列实现落点（表）。

## 微调

- `overview.md`：开头或末尾加「理论来源」段，链到 `theory.md` + 4 篇来源文档路径。
- `README.md`：索引表加 `theory.md`、`gap-analysis.md` 两行。

## 决策

- **D1**：理论文档命名为 `theory.md`（不是 `design-rationale.md`），简短好记。
- **D2**：gap 分析作为正式文档放 `docs/wiki/gap-analysis.md`（用户要「详细写 wiki 模块」，gap 是模块的一部分），而非只留在 plantree topic 里。
