# 0003 plan-tree 全部能力整体迁入项目域，markdown 可导入

**日期**：2026-09-04　**状态**：已拍板

## 背景

开发类项目的跨会话上下文目前靠 plan-tree（docs/plantree/ markdown 规划树）硬撑。用户明确了未来意图：agent-memory 里要有 plan-tree 的全部能力，从开发角度这是一个「可迁移的东西」。

## 决策

**plan-tree 的能力整体迁入 agent-memory 项目域**，不是共存：

- plan-tree 核心概念成为项目域的一等公民：

| plan-tree 概念 | 项目域身份 |
|---|---|
| roadmap | 项目计划层（阶段/里程碑） |
| decisions | 决策记录（含 rationale） |
| evidence | 条目的证据挂靠 |
| status / plan | 进度状态 |
| baseline | 项目基线（起点快照） |
| topics | 项目内分区（「类型决定骨架」的落点） |

- **迁移路径留好**：现有 `docs/plantree/` 的 markdown 要能导入（plan-tree 结构规整，导入器可行）。
- plan-tree skill 未来退役或退化成薄壳（转调 agent-memory），文件层收编进库。

## 硬约束（对数据模型）

项目域数据模型必须**一开始就按 plan-tree 概念设计**，不是事后贴分区标签——否则迁移导入时装不下 roadmap/decisions/evidence 这些结构。

## 后果

- 开发类型的骨架 ≈ 结构化 plan-tree（见 [0002](0002-two-types-first.md)）。
- 本根（project-memory plan）本身将来也要迁入项目域（吃狗粮闭环）。
- plan-tree skill 的退役时机：项目域能力覆盖其核心工作流之后，另行决策。
