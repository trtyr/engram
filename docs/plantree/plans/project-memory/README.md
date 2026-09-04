# Project Memory（项目记忆域）

**状态：Planning（概念对齐完成 2026-09-04，待设计）**

agent-memory 第五域：围绕长期任务的跨会话工作上下文。本根记录概念对齐结论与后续设计。

## Scope

- 概念：**项目 = 一件有明确目标、一次干不完、跨多次会话推进、有状态演化的工作单元**。项目记忆记的是「线」（目标+进度+决策+资料聚合），区别于现有四域记的「点」（用户事实/资料/知识页/代码结构）。
- 类型先行两种：**开发** + **调研**（见 decisions）。
- 长期承载 plan-tree 全部能力（见 decisions）。

## Authority & File Map

- [roadmap.md](roadmap.md) —— 阶段与状态
- [decisions/](decisions/) —— 已拍板决策（编号制）
- [open-questions.md](open-questions.md) —— 未决设计问题

## 背景一句话

现状痛点：调研/开发的过程记忆散在 knowledge chunks、会话历史、plan-tree 文件三处，没有「这件事」的锚点；换会话接不上，靠翻历史硬拼。项目记忆把这种「拼回来的功夫」变成系统一等公民。

相关边界系统：project-manage（人的项目管理，不动）、learnsys（学习，不动）、plan-tree skill（未来迁入或退化薄壳）。
