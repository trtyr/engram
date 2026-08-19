# Plan Tree — agent-memory

单用户 AI 长期记忆平台：平台提供 HTTP API，AI 拿着 API 操纵平台；
人通过 Web UI 管理浏览。Rust 后端 + React/Vite 前端 + Docker 交付。

## 权威顺序（Authority Order）

1. **用户最新指示** —— 高于一切文档
2. **decisions/** —— 已拍板的稳定决策，冲突时以此为准
3. **roadmap.md** —— 当前执行路线
4. **topics/** —— 领域设计细节
5. **open-questions.md** —— 未决问题（不是任务）
6. **baseline/** —— 项目级背景

## 目录结构

```text
docs/plantree/
├── README.md                 ← 你在这里：注册表
├── baseline/                 ← 项目级背景
│   ├── README.md             ← 使命、范围、技术栈基线、参考项目
│   ├── module-map.md         ← 目标模块地图（server crates + web）
│   ├── runtime-flows.md      ← 四条核心流程（蒸馏/摄取/Wiki/图谱）
│   ├── storage-and-state.md  ← PG schema 概要与文件布局
│   ├── test-and-release-gates.md ← 质量门与交付验收
│   └── risk-hotspots.md      ← R1–R9 风险表
├── plans/
│   └── agent-memory-platform/   ← 主计划（唯一活跃计划）
│       ├── README.md            ← 计划范围与文件地图
│       ├── roadmap.md           ← 八阶段总览与状态
│       ├── phases/              ← phase-0 … phase-7 工作分解
│       ├── topics/              ← 11 份领域设计
│       ├── decisions/README.md  ← D0001–D0008
│       └── open-questions.md    ← Q1–Q7
└── ideas/inbox.md            ← 低承诺想法池
```

## 活跃计划表

| Plan | Status | Current Phase | Last Landed | Next Target |
|---|---|---|---|---|
| [agent-memory-platform](plans/agent-memory-platform/README.md) | In Progress | Phase 1 核心底座 | Phase 0 地基完成 (2026-10) | Phase 1 schema/jobs/llm/鉴权 |

## 如何读本树

- **第一次来**：[baseline/README.md](baseline/README.md)
  → [plans/agent-memory-platform/README.md](plans/agent-memory-platform/README.md)
  → [roadmap.md](plans/agent-memory-platform/roadmap.md)
- **恢复执行**：roadmap.md → 目标 phase 文件 → 相关 topics/
- **查为什么这么设计**：plans/agent-memory-platform/decisions/
- **查某领域怎么做**：plans/agent-memory-platform/topics/

## 基线引用

所有计划默认继承 [baseline/](baseline/README.md) 的项目级约束，不重复抄写。
