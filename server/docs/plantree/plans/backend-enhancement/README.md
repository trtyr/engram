# backend-enhancement

后端增强路线图：**现状 vs 目标**的差距清单，拆成「智能深度 + 工程化健壮性」两块，产出可落地的优化与功能添加。

## Scope

- **In**：后端优化 + 功能添加，让项目更强（检索、蒸馏、质量、性能、可观测、安全、错误处理、API、测试、运维）。
- **Out**：团队功能（多用户/多 Agent、loadout、ACL、可见性）——用户明确不要，项目定位单用户。
- **Out**：本计划只做规划（清单 + 依据），**不实现代码**；实施是否另起计划由用户决定。

## Authority

- 现状事实：以 `crates/` 源码为准（本轮已盘点 memory/knowledge/wiki/codegraph/llm/jobs/search/storage/parsing/distill）。
- 理论依据：用户知识库 4 篇 wiki 设计文档（Karpathy 原文 + TencentDB + llm_wiki + 评论区实战教训）。

## File Map

| 文件 | 角色 |
|---|---|
| `roadmap.md` | 核心清单（P0/P1/P2 分优先级，每项「现状→目标」） |
| `topics/retrieval.md` | 检索智能方向展开 |
| `topics/distillation.md` | 蒸馏/质量方向展开 |
| `topics/engineering.md` | 工程化健壮性方向展开 |
