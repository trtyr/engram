# Baseline — 项目背景

## 现状

新项目，仓库为空（`Unknown / needs inventory`：无既有代码、无既有依赖）。本 baseline 记录的是**目标架构基线**，随实现落地逐步校正为事实。

## 使命

**把长期记忆做成 AI 的一个工具（skill）。**

- 平台提供 HTTP API，把 API 交给 AI，由 AI 操纵 API 控制平台。
- 平台自身**内置 LLM 蒸馏管道**：原始经验进（L0），分层蒸馏出可用记忆（L1/L2/L3）。
- 人通过 Web UI 浏览、管理、纠偏；AI 通过 API 高频读写。同一个真相源。

## 范围

### 做（四类记忆资产）

| 资产 | 内容 | 主要参考 |
|---|---|---|
| Chat Memory | L0 会话 → L1 原子 → L2 场景 → L3 用户画像，分层蒸馏 | TencentDB-Agent-Memory |
| Knowledge | 文档/URL 摄取 → 解析分块 → 嵌入 → 混合检索 | TDAM + 通用 RAG |
| Wiki | Karpathy 模式：sources 不可变 → LLM 维护 wiki 页 + 链接图 | nashsu/llm_wiki |
| CodeGraph | 代码知识图谱（**复用 colbymchenry/codegraph 现成能力**，平台包装代理） | codegraph |

### 不做（硬边界）

- ❌ 短期记忆（另有系统负责）
- ❌ 团队 / 多租户 / ACL 权限体系（单用户）
- ❌ MCP server（只做 HTTP API）
- ❌ 自己实现代码解析器（CodeGraph 复用现成项目）

## 技术栈基线

| 层 | 选型 | 决策 |
|---|---|---|
| 后端 | Rust + axum + sqlx + utoipa | [D0006](../plans/agent-memory-platform/decisions/README.md) |
| 存储 | PostgreSQL 17 + pgvector（向量）+ FTS（全文）+ jsonb | [D0003](../plans/agent-memory-platform/decisions/README.md) |
| LLM | 内置蒸馏管道，多 provider（OpenAI 兼容），模型路由 | [D0002](../plans/agent-memory-platform/decisions/README.md) |
| AI 接入 | 纯 HTTP REST + Bearer API key | [D0004](../plans/agent-memory-platform/decisions/README.md) |
| 前端 | React 19 + Vite + TS + Tailwind + shadcn/ui + Zustand + TanStack Query | [D0006](../plans/agent-memory-platform/decisions/README.md) |
| CodeGraph | 包装 codegraph CLI（npm 自带 runtime），子进程调用 | [D0005](../plans/agent-memory-platform/decisions/README.md) |
| 交付 | Docker 多阶段构建 + docker-compose（app + pgvector） | — |

## 参考项目

| 项目 | 借鉴点 |
|---|---|
| [TencentCloud/TencentDB-Agent-Memory](https://github.com/TencentCloud/TencentDB-Agent-Memory) | L0-L3 分层蒸馏、资产模型、混合检索（BM25+向量+RRF）、上下文预算控制 |
| [nashsu/llm_wiki](https://github.com/nashsu/llm_wiki) | Karpathy Wiki 模式、两步 ingest、wikilink 图、SHA 增量缓存、lint |
| [chandra447/pi-hermes-memory](https://github.com/chandra447/pi-hermes-memory) | 记忆分类学（failure/correction/insight/preference/convention）、correction 捕获、自动整理 |
| [colbymchenry/codegraph](https://github.com/colbymchenry/codegraph) | 整个模块直接复用：tree-sitter 解析 + SQLite 符号图 + explore/callers/callees/impact |

## 产品成熟度要求

用户明确要求：**一次做成熟产品，不做 demo**。

- 不留「临时方案以后替换」：第一行代码就是生产路径（真 PG、真迁移、真测试、真 Docker）。
- 阶段划分按领域垂直切片，每个阶段交付的都是该领域可用的完成态，不是骨架。
- 详见 [test-and-release-gates.md](test-and-release-gates.md)。
