# 概览

**Engram server** 是单用户 AI 长期记忆平台的后端。前端（Engram 控制台）与第三方 AI Agent
都通过它的 HTTP API 工作。

## 解决什么问题

AI Agent 的记忆是易失的：会话结束即丢。本服务把记忆做成可蒸馏、可检索、可审计的持久资产：

```
L0 会话（raw_sessions）        Agent 对话原文，逐轮 speaker/text
  │ 蒸馏（extract→arbitrate→organize→consolidate）
L1 原子（atoms）               事实/偏好/技能级知识单元，带置信度/人审/敏感/时间标记
  │ 组织
L2 场景（scenarios）           原子聚合成可复用的场景模式（快照收敛：成员归档则重算/解散）
  │ 沉淀
L3 画像（persona_aspects）     稳定的用户分面画像，供 prompt 注入（清退>钉住>自动重写）
  ↕ atom_entities（蒸馏自动抽取）
entities（记忆坐标系）          人物/项目/主题/群组/地点——横向切记忆的透镜
```

旁路三条资产线：

- **知识库**：文档上传→解析（pdf/docx/html/md/txt）→分块→embedding（pgvector）→语义检索
- **LLM Wiki**：多源摄取→LLM 分析（实体/主题/链接）→页面生成→版本演进→链接图谱+社区发现
- **CodeGraph**：注册代码库→codegraph CLI 索引→结构化查询（cg-bridge 桥接）

横切设施：任务队列（jobs，带重试/事件流水/revive/cancelled）、LLM 网关（多 provider 加密密钥+路由表+用量记账）、
统一跨域检索（/search 融合记忆/知识/Wiki/实体四域）。

治理能力（2026-08-31 起）：敏感标记全链排除（检索/打包/导出/快照生成输入）、
一等清空（agent 可逆归档 / deep 两阶段+后悔药）、编辑分权与留痕、全量导出。

## 整体形状

```
┌─────────────────────────── engram-api (bin) ───────────────────────────┐
│  axum Router                                                                │
│  ├─ 公开: /health /ready /openapi.json /auth/login                          │
│  ├─ Bearer 认证层（admin 会话 ams_ / api key amk_；/jobs 对 text/html 分流 SPA）│
│  └─ 9 个域路由: auth / memory / knowledge / wiki / codegraph / jobs /        │
│                llm(settings) / search / health                              │
│  rust-embed ── web/dist 同源托管（生产单二进制）                              │
└──────┬───────────────────────────────────────────────────────────────────────┘
       │ sqlx
┌──────▼──────────────┐   ┌──────────────┐   ┌────────────────────────────┐
│ PostgreSQL + pgvector│   │ JobQueue     │   │ LLM 网关（reqwest→外部 API）│
│ 27 业务表 / 28 迁移   │   │ 死信/重试/事件 │   │ 密钥加密/路由/用量          │
└─────────────────────┘   └──────────────┘   └────────────────────────────┘
```

详细模块地图见 [architecture.md](architecture.md)。
