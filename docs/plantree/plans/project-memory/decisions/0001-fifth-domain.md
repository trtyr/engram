# 0001 项目记忆长在 agent-memory 做第五域，不单独建系统

**日期**：2026-09-04　**状态**：已拍板

## 背景

项目记忆能力有三个落点可选：agent-memory 新增第五域 / 单独写一个项目记忆系统 / 挂到 project-manage。用户倾向 agent-memory，但对「单独建系统」犹豫（原话：「这个有点太分权了」）。

## 决策

**项目记忆长在 agent-memory 里，做第五个域**（projects + 项目条目 + 类型骨架），与 memory / knowledge / wiki / codegraph 并列。

## 理由

1. **内容大头本来就在这**：一个项目的记忆 = 资料（knowledge documents）+ 知识点（wiki pages）+ 事实与决策（memory atoms）。项目是把这些点串成线的壳；单独建系统则每次挂资料都要跨系统引用，最贵且易漂移。
2. **跨会话接续是「AI 的记忆」问题**：agent-memory 定位就是 AI 的长期记忆器官（memory.py + skill 是入口）；单独建系统会把 AI 记忆器官裂成两个。
3. **避免重复建设**：DB/API/鉴权/Web/CI 全套再来一遍，且已有 project-manage 一个独立系统，再加就是第三个管「项目」的东西。

## 边界（随之确定）

| 系统 | 管什么 | 视角 |
|---|---|---|
| agent-memory · 项目域 | AI 干活的工作上下文：进度、决策、资料挂靠、跨会话接续 | AI 的记忆 |
| project-manage | 人的项目管理：客户、人员、任务分配、交付物 | 人的管理 |
| learnsys | 学习项目：卡片、复习、路径 | 学习专用 |

## 后果

- agent-memory 增加项目域的表结构、API、Web 页面与 AI 侧接口。
- project-manage / learnsys 不动。
