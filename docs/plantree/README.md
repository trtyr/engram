# Plan Tree

Engram 仓库的规划树根。项目级基线由 [baseline/](baseline/README.md) 承担；完整项目档案在 [docs/](../README.md)（project-init 全栈归档 + 设计审计与 Engram 重设计证据）。

## Active Plans

| Plan | Status | Current Phase | Last Landed | Next Target |
|---|---|---|---|---|
| [frontend-polish](plans/frontend-polish/README.md) | In Progress | R1+R2 已落地并推送（ffc5568 双绿） | 双 P1 修复 + /jobs Accept 分流 + 侧栏四件套（收缩/徽章/检索/分区）+ 六轮视觉修复 | R3 移动端与可达性 |
| [memory-rhythm](plans/memory-rhythm/README.md) | Done | 双节律收官（cron scope 分权 d7a8345 + 落档 8db2c36） | 外部 cron + 层 A+B + 心跳/status + 三线分权 + 设置节律页 | 上线后观察：conflict-matrix B/D 项 |
| [circle](plans/circle/README.md) | Done | 13 项强化全量落地（2026-09-01 goal mtiojzx1） | 审计 + P1/P2/P3 实施：关系升级 A/时间轴/批量/导出/检索/去重/历史 | 数据密度自然积累 |
| [ai-permissions](plans/ai-permissions/README.md) | Done | 权限收窄 + 二期三项全落地验收（2026-09-02） | 收窄直写/erase 分权 + 会话敏感 + 文件导入 + 过期降权 + 时间过滤 | 稳定运行（Deferred 观察项） |
| [wiki-unify](plans/wiki-unify/README.md) | Planning | Knowledge+Wiki 合并方案已成（2026-09-02，用户拍板：彻底合一/保留 Wiki/自动织 + Obsidian 式知识图谱） | 无（规划中） | 等 open-questions 拍板后实施：后端合一 → 自动织 → 前端融合 → 图谱升级 |
| [project-memory](plans/project-memory/README.md) | Planning | 开发项目详情模型已对齐（2026-09-04 二轮：三表+类型=分类模板+plan-tree 消融为规划分类，五决策落档） | 无（规划中） | 数据模型 DDL + API 设计 |

## Registered Roots

- `docs/plantree/`（本根）——2026-08-30 引导建立。
- **遗留树**：[server/docs/plantree/](../../server/docs/plantree/README.md)——后端规划树（backend-enhancement 等），**未迁移**；后端规划继续在那里维护。

## How to Read

1. 读目标 plan root 的 `README.md`（scope + authority + file map）
2. 读 `roadmap.md`（Done / In Progress / Next / Deferred）
3. 读 `topics/`（方案胶囊）与 `open-questions.md`（未决）
