# P002 · MCP 工具面修复线（agent-feedback 工单批）

> 目标：消化 engram 使用中发现的 MCP 工具面问题（工单 EN-10 / EN-11 / EN-12 / EN-24 / EN-25）+ 工单台账 hygiene。
> 问题记录权威：tickets 域各工单（EN-xx 原文）；本树只追修复状态与操作上下文，不复制工单全文。

## Scope

**In**：上述 5 张工单的定位与修复；重复单归档 / 过期待办销账等台账整理。
**Out**：P001 风险债 22 项（另一条线）；工具面之外的产品语义变更；新功能。

## Affected Modules

`server/mcp`（本仓库；hygiene 任务不涉码）——**外部组件不归本线**（2026-10-01 拍板：查明问题由 engram 之外的组件所致时，只记录证据并转出/关闭工单，不动外部）

## 文件地图（阅读路径）

1. [roadmap.md](roadmap.md) — 任务状态（唯一任务权威，含建议修复序）
2. [topics/tooling-fix-map.md](topics/tooling-fix-map.md) — 各单修复入口 / 验证路径操作卡
3. [open-questions.md](open-questions.md) — **等你拍板的**

## 执行纪律

- **管辖边界（2026-10-01 拍板）**：只修 engram 仓库内的问题，外部组件不碰——T002 直接在 engram 工具面路径排查，证据指向外部则记录 + 工单转出/关闭
- 改 MCP 工具面必须过 `tests/golden/mcp_surface.json` 快照 + dispatch.rs 读写分类护栏（见 baseline/test-and-release-gates.md）
- 纯 hygiene 动作（工单 link / 归档 / todo 销账）无争议直接做，落地后 roadmap 记证据
- ~~EN-10 修复前：档案 / 文档维护**避免使用 doc_patch replace_text**~~（**已解除** 2026-10-01：EN-10 已修复 `576e57c`——content 缺失/空现显式报错，回执带 old_chars/new_chars 可自查；hint 提醒回执不可替代 doc_get 回读验证，长文patch 后仍建议回读）
