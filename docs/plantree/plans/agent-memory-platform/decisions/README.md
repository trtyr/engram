# Decisions — 已拍板决策

决策一经记录不轻改；推翻某决策需新增决策文件说明替代关系。

## D0001 平台即工具，只做长期记忆

- **状态**：已确认（用户，2026-10）
- **背景**：核心理念——平台提供 API，AI 操纵 API 控制平台；短期记忆另有系统负责。
- **决策**：HTTP API 是第一公民，为人服务的 Web UI 消费同一套 API。不设计任何会话内/工作记忆能力。
- **后果**：API 必须自描述（OpenAPI + 面向 AI 的接入文档）；平台不做 chat、不做 prompt 管理。

## D0002 内置 LLM 蒸馏管道

- **状态**：已确认（用户，2026-10）
- **背景**：纯工具型（AI 自己分步蒸馏）vs 内置管道（平台调 LLM 自动蒸馏）。
- **决策**：平台内置蒸馏管道：L0→L1→L2→L3 与 Wiki ingest 均由服务端 LLM 完成。多 provider（OpenAI 兼容），按任务类型路由模型，密钥服务端加密，用量记账。
- **被否方案**：纯工具型（AI 客户端 token 成本高、质量不稳定）；混合任务编排型（复杂度收益不成比例）。

## D0003 PostgreSQL 17 + pgvector

- **状态**：已确认（用户，2026-10）
- **决策**：单 PG 引擎承担关系数据 + pgvector 向量 + FTS 全文。sqlx 迁移唯一定义 schema。
- **被否方案**：SQLite + sqlite-vec（并发与向量规模弱）；SQLite + LanceDB（双引擎维护成本）；PG 换 MySQL（pgvector 生态最成熟）。

## D0004 只做 HTTP API，不做 MCP

- **状态**：已确认（用户，2026-10）
- **决策**：AI 接入只走 HTTP REST（Bearer API key）。随平台交付「AI 接入说明」（可直接粘贴给 agent 的 API 用法文档），弥补无 MCP 的接入摩擦。
- **后果**：MCP wrapper 若有需求，将来作为独立薄层叠加（见 ideas）。

## D0005 CodeGraph 复用现成项目

- **状态**：已确认（用户，2026-10）
- **决策**：不重新实现代码解析。以子进程方式包装 colbymchenry/codegraph CLI（`--json` 输出），平台侧做项目注册/同步/查询代理与错误归一。Docker 镜像内安装 codegraph（自带 Node runtime）。
- **后果**：镜像变大、上游版本需 pin；解析质量与语言覆盖继承上游。

## D0006 技术栈定型

- **状态**：已确认（用户，2026-10）
- **决策**：后端 Rust：axum + sqlx + utoipa + tokio；monorepo `server/crates/*`（见 [baseline/module-map](../../../baseline/module-map.md)）。前端 React 19 + Vite + TypeScript + Tailwind + shadcn/ui + Zustand + TanStack Query，类型从 OpenAPI 生成。交付 Docker 多阶段单镜像 + compose。

## D0007 单用户，无团队/多租户

- **状态**：已确认（用户，2026-10）
- **决策**：不做 team/tenant/ACL。认证 = 管理员密码（Web UI）+ API key（AI 客户端）。全部表无租户列。

## D0008 成熟产品标准，一次到位

- **状态**：已确认（用户，2026-10）
- **决策**：不做 demo/临时方案。每阶段出口即生产质量：真 PG、真迁移、真测试、真 Docker、文档同步。阶段 = 领域垂直切片，而非水平分层。
- **后果**：阶段出口标准见 [baseline/test-and-release-gates](../../../baseline/test-and-release-gates.md) 与各 phase 文件。
