# P011 · memory 实读行为债（2026-10-03 全链审计）

## 范围
2026-10-03 对用户记忆域全链实读（repo/memory 七件套 + core/memory 五件 + distill 十四文件
+ MCP dispatch 动作面，约 7300 行）发现的**行为/代码层问题**：正确性裂缝、性能债、卫生债、
结构观察。**文档偏差不在本线**（用户拍板「先不管文档」，档案修正另案）。

## 权威
- 实读证据：2026-10-03 会话内逐文件实读，每条带 文件:行号（roadmap.md）
- 交叉：蒸馏链篇坑清单第 2/3/4/5/6 条全部实证为真；P001 Deferred 已覆盖的
  （prompt_version 归因列 / 归并 LIMIT 1 ORDER BY）**不重复开任务**，见 roadmap Deferred。

## 关键事实（为什么值得开线）
- 检索主路径 2026-09-12 起敏感全放开（core/search.rs:97 注释），reveal 机制已移除——
  多处注释与文档还停在旧口径（T009）。
- P010 定了「日志是唯一时间线」，但审计事件（void/erase/correct/merge 等 11+ 处）
  走 repo/memory/ops.rs:88 `audit()` 直插 jobs 表伪造 succeeded job——logs 流不可见。
- 场景成员双轨（atom_refs 并集 vs scenario_id 覆盖回填）存在真实漂移路径，
  目前仅靠 converge 收敛兜底。

## 文件
- roadmap.md：任务与状态（当前全部 Planning，待拍板）
- open-questions.md：Q001-Q004 待拍板
