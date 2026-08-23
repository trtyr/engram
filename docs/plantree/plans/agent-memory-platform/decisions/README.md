# Decisions — 已拍板决策

决策一经记录不轻改；推翻某决策需新增决策文件说明替代关系。

## D0009 中文检索：应用层 jieba 预分词 + pg_trgm 补位（原 Q1）

- **状态**：已确认（2026-08，Phase 1 开工时按 plan 倾向落定）
- **决策**：写入与查询两侧均用 jieba-rs 切词后以空格拼接写入 tsvector（`simple` 配置）；
  pg_trgm 作为子串匹配补充；向量检索兜底语义召回。不引入 zhparser（免自编译扩展）。
- **后果**：search crate 依赖 jieba-rs；tsv 列由应用层维护（无触发器）。

## D0010 embedding 定为 Qwen3-Embedding-8B / 1024 维（原 Q2）

- **状态**：已确认（2026-08，schema 落地前定案；Phase 1 末按真实网关修订）
- **决策**：存储统一 `vector(1024)`。默认 embedding 通道 Qwen3-Embedding-8B
  （用户网关实际供给，原生 4096 维）以 matryoshka `dimensions=1024` 降维调用。
  换模型属破坏性迁移（重新嵌入全量数据）；路由层允许配其他模型但维度必须 1024 兼容。
- **修订记录**：初版倾向 bge-m3；真实网关探测（evidence Phase 1）后改为 Qwen3-Embedding-8B。


## D0012 验证口径：本地二进制优先，Docker 仅打包（2026-08-20，用户决策）

- **状态**：已确认（用户，2026-08-20）
- **决策**：开发与验证循环全部使用本地二进制（cargo run + rust-embed dist + 本地
  PG/testcontainers）；Docker 镜像仅作为最终交付打包产物（build 通过即可），不作为
  验证对象（同一二进制 + 同源前端，行为等价）。
- **背景**：连续多轮 Docker 构建受宿主网络（Clash TUN fake-ip / npm 代理兼容 / apt 源）
  干扰，构建循环慢且引入与产品无关的失败；用户明确改为本地打磨完再打包。
- **后果**：phase-6/7 出口标准措辞已同步；e2e/evidence 以本地栈输出为准。

## D0011 管理员会话用 opaque token + PG 表（原 Q6）

- **状态**：已确认（2026-08，Phase 1 实现前）
- **决策**：登录颁发随机 opaque token（sha256 哈希存 admin_sessions 表，带过期与
  last_used_at），可随时吊销；不用 JWT。单用户场景最简且可控。

## D0001 平台即工具，只做长期记忆

- **状态**：已确认（用户，2026-08）
- **背景**：核心理念——平台提供 API，AI 操纵 API 控制平台；短期记忆另有系统负责。
- **决策**：HTTP API 是第一公民，为人服务的 Web UI 消费同一套 API。不设计任何会话内/工作记忆能力。
- **后果**：API 必须自描述（OpenAPI + 面向 AI 的接入文档）；平台不做 chat、不做 prompt 管理。

## D0002 内置 LLM 蒸馏管道

- **状态**：已确认（用户，2026-08）
- **背景**：纯工具型（AI 自己分步蒸馏）vs 内置管道（平台调 LLM 自动蒸馏）。
- **决策**：平台内置蒸馏管道：L0→L1→L2→L3 与 Wiki ingest 均由服务端 LLM 完成。多 provider（OpenAI 兼容），按任务类型路由模型，密钥服务端加密，用量记账。
- **被否方案**：纯工具型（AI 客户端 token 成本高、质量不稳定）；混合任务编排型（复杂度收益不成比例）。

## D0003 PostgreSQL 17 + pgvector

- **状态**：已确认（用户，2026-08）
- **决策**：单 PG 引擎承担关系数据 + pgvector 向量 + FTS 全文。sqlx 迁移唯一定义 schema。
- **被否方案**：SQLite + sqlite-vec（并发与向量规模弱）；SQLite + LanceDB（双引擎维护成本）；PG 换 MySQL（pgvector 生态最成熟）。

## D0004 只做 HTTP API，不做 MCP

- **状态**：已确认（用户，2026-08）
- **决策**：AI 接入只走 HTTP REST（Bearer API key）。随平台交付「AI 接入说明」（可直接粘贴给 agent 的 API 用法文档），弥补无 MCP 的接入摩擦。
- **后果**：MCP wrapper 若有需求，将来作为独立薄层叠加（见 ideas）。

## D0005 CodeGraph 复用现成项目

- **状态**：已确认（用户，2026-08）
- **决策**：不重新实现代码解析。以子进程方式包装 colbymchenry/codegraph CLI（`--json` 输出），平台侧做项目注册/同步/查询代理与错误归一。Docker 镜像内安装 codegraph（自带 Node runtime）。
- **后果**：镜像变大、上游版本需 pin；解析质量与语言覆盖继承上游。

## D0006 技术栈定型

- **状态**：已确认（用户，2026-08）
- **决策**：后端 Rust：axum + sqlx + utoipa + tokio；monorepo `server/crates/*`（见 [baseline/module-map](../../../baseline/module-map.md)）。前端 React 19 + Vite + TypeScript + Tailwind + shadcn/ui + Zustand + TanStack Query，类型从 OpenAPI 生成。交付 Docker 多阶段单镜像 + compose。
- **修订注记（2026-08，Q10）**：Zustand 声明于栈内但落地时 src 无任何引用（UI 状态用
  React 本地 state + TanStack Query 已覆盖），已从 package.json 移除并同步 AGENTS.md 约定。

## D0007 单用户，无团队/多租户

- **状态**：已确认（用户，2026-08）
- **决策**：不做 team/tenant/ACL。认证 = 管理员密码（Web UI）+ API key（AI 客户端）。全部表无租户列。

## D0008 成熟产品标准，一次到位

- **状态**：已确认（用户，2026-08）
- **决策**：不做 demo/临时方案。每阶段出口即生产质量：真 PG、真迁移、真测试、真 Docker、文档同步。阶段 = 领域垂直切片，而非水平分层。
- **后果**：阶段出口标准见 [baseline/test-and-release-gates](../../../baseline/test-and-release-gates.md) 与各 phase 文件。
