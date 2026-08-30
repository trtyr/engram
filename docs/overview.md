# 概览

**agent-memory（产品名 Engram）**——单用户 AI 长期记忆平台。
一个仓库、两个 workspace、三层档案：本目录是全栈集成视角，细节下沉到
[server/docs/](../server/docs/README.md)（后端项目）与 [web/docs/](../web/docs/README.md)（前端项目）。

## 它做什么

AI Agent 的记忆不该随会话蒸发。Engram 把记忆做成**可蒸馏、可检索、可审计**的资产：

```text
L0 会话 ──蒸馏──▶ L1 原子 ──组织──▶ L2 场景 ──沉淀──▶ L3 画像
（原文）          （事实单元）      （场景模式）       （prompt 注入用画像）
```

三条旁路资产线：**知识库**（文档→分块→向量检索）、**LLM Wiki**（多源摄取→生成→图谱）、
**CodeGraph**（代码库结构索引）。横切：任务队列（死信/重试/事件）、LLM 网关（多 provider
加密密钥/路由/用量）、跨域统一检索。

## 谁在用

1. **人**——Engram 控制台（web/，七域 SPA，墨白双主题）
2. **AI Agent**——API Key 调 /memory/context 取画像与相关记忆，写会话回灌

## 仓库形状

```text
agent-memory/
├── server/          # Rust workspace（10 crates，axum+sqlx+pgvector，rust-embed 托管前端）
│   └── docs/        # ← 后端项目档案
├── web/             # React 19 SPA（Engram 控制台）
│   └── docs/        # ← 前端项目档案
├── docs/            # ← 本档案（全栈集成视角）
│   ├── design/      #   设计审计证据（audit.md + 截图 + 度量）
│   └── plantree/    #   规划树（frontend-polish 进行中）
├── deploy/          # Dockerfile + docker-compose（开发期暂不维护，CI 验证构建）
├── scripts/         # e2e Python 脚本、备份、provider 验证
├── PRODUCT.md       # 产品事实（impeccable 产品文档）
└── DESIGN.md        # Engram 设计系统（视觉世界/token/禁区）
```

## 技术形态一句话

Rust(axum) 单二进制同源托管 React SPA；PostgreSQL+pgvector 存储；LLM 调用全部任务化异步。
详细：[architecture.md](architecture.md)。
