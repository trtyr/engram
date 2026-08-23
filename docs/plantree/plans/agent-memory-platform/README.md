# Plan — agent-memory-platform

单用户 AI 长期记忆平台的完整交付计划。目标：一次做成熟产品，不做 demo。

## 范围

四类记忆资产（Chat Memory 分层蒸馏 / Knowledge / Wiki / CodeGraph 代理）+ 内置 LLM 蒸馏管道 + 纯 HTTP API + Web 管理控制台 + Docker 交付。

边界与项目级约束继承 [baseline](../../../baseline/README.md)，不在此重复。

## 文件地图

| 文件 | 角色 |
|---|---|
| [roadmap.md](roadmap.md) | 路线图：八阶段总览与当前状态 |
| [phases/](phases/) | 各阶段详细工作分解与验收标准 |
| [decisions/README.md](decisions/README.md) | 已拍板决策（D0001–D0012） |
| [open-questions.md](open-questions.md) | 未决问题 |
| [topics/api-design.md](topics/api-design.md) | API 全目录与契约约定 |
| [topics/memory-model.md](topics/memory-model.md) | L0–L3 记忆模型 |
| [topics/distill-pipeline.md](topics/distill-pipeline.md) | 蒸馏管道设计 |
| [topics/knowledge-ingest.md](topics/knowledge-ingest.md) | 知识摄取管道 |
| [topics/wiki-engine.md](topics/wiki-engine.md) | Wiki 引擎设计 |
| [topics/codegraph-bridge.md](topics/codegraph-bridge.md) | CodeGraph 桥接 |
| [topics/search.md](topics/search.md) | 混合检索设计 |
| [topics/jobs-system.md](topics/jobs-system.md) | 任务系统设计 |
| [topics/llm-providers.md](topics/llm-providers.md) | LLM 提供方与路由 |
| [topics/frontend.md](topics/frontend.md) | 前端信息架构 |

## 阅读路径

- **执行某一阶段**：roadmap.md → phases/phase-N.md → 该阶段涉及的 topics/
- **查设计依据**：decisions/README.md
- **查未定项**：open-questions.md

## 状态

**Done（v0.1.0，2026-08-20 独立完成审计批准）**。八阶段全部交付，后续见
roadmap.md Done 区、[open-questions.md](open-questions.md)（Q8–Q11 初始化审计项）与
[evidence/README.md](evidence/README.md)。
