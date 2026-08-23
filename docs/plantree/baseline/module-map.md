# Module Map — 实际模块地图

> 现状：八阶段全部落地（v0.1.0）。本文件已按实际结构调整过——目标架构期间
> api 仅经 core 访问域服务，落地后 wiki/codegraph 域由 api 直连（见「与目标架构的偏差」）。

## 仓库布局（monorepo）

```
agent-memory/
├── server/                    # Rust 后端（axum 工作区）
│   ├── Cargo.toml             # workspace root
│   ├── crates/
│   │   ├── api/               # HTTP 层：路由、handler、DTO、OpenAPI、中间件（auth/error/log）
│   │   ├── core/              # 领域层：memory/knowledge/wiki/codegraph 各域服务
│   │   ├── storage/           # sqlx 仓储层：迁移、查询、事务
│   │   ├── llm/               # LLM 客户端：provider 抽象、模型路由、用量统计
│   │   ├── jobs/              # 任务系统：PG-backed 队列、worker、调度、事件
│   │   ├── distill/           # 蒸馏管道：L0→L1→L2→L3 编排、提示词模板
│   │   ├── wiki-engine/       # Wiki 引擎：两步 ingest、wikilink、lint、图数据
│   │   ├── cg-bridge/         # CodeGraph 桥：CLI 子进程包装、JSON 解析、项目注册
│   │   └── search/            # 混合检索：FTS + 向量 + RRF 融合、预算控制
│   └── migrations/            # sqlx 迁移（唯一定义处）
├── web/                       # React + Vite 前端
│   └── src/
│       ├── features/          # dashboard/memory/knowledge/wiki/codegraph/jobs/settings
│       ├── components/        # 共享 UI（shadcn/ui 定制）
│       ├── lib/               # API client、类型（从 OpenAPI 生成）
│       └── stores/            # Zustand
├── deploy/                    # Dockerfile、docker-compose.yml、.env.example
└── docs/plantree/             # 本计划树
```

## 依赖方向（单向，实际）

以各 crate 的 Cargo.toml 为准（2026-08 初始化审计核实）：

```text
api → (storage, jobs, llm, core, distill, wiki-engine, cg-bridge)
core → (jobs, llm, search, distill)
storage → (sqlx only：pool 装配 + 迁移执行)
```

- 域逻辑默认放 `core`：memory/knowledge 已在此（core 内含 sqlx 查询）。
- **与目标架构的偏差**：目标为 `api → core → (各域)`，落地后 `api` 直接消费
  `wiki-engine`（WikiService）与 `cg-bridge`（CgBridge），并直接依赖 `jobs`/`llm`/
  `distill`/`storage`。`storage` 仅为 pool+迁移薄层，未建仓储抽象层。
  已记录为开放项（open-questions.md 初始化审计节），收敛回 core 需重构，非紧急。
- `llm`/`jobs` 不依赖任何内部 crate。
- 禁止域间横向 import（`distill` 不 import `wiki-engine`）；跨域协作放 `core` 编排。

## 模块职责一句话

| 模块 | 职责 | 深模块接口要点 |
|---|---|---|
| api | HTTP 契约层 | 路由注册、DTO↔领域转换、统一错误体、OpenAPI |
| core | 领域编排 | memory/knowledge 域服务（含 sqlx 查询）+ 任务触发 |
| storage | 持久化薄层 | pool 装配 + 迁移执行（无仓储 trait，查询在 core/api 直接用 sqlx） |
| llm | 模型访问 | `trait Provider`、路由规则（任务类型→模型）、加密密钥、用量记账 |
| jobs | 异步执行 | job 状态机、重试策略、事件流（SSE 源）、幂等键 |
| distill | 蒸馏编排 | 提示词模板版本化、L0→L3 各阶段任务、consolidation |
| wiki-engine | 知识编译 | 两步 ingest、frontmatter/wikilink 解析、index/log 维护、lint |
| cg-bridge | 图谱代理 | codegraph CLI 调用、超时/错误归一、项目生命周期 |
| search | 统一检索 | 单入口 `search(query, scopes, budget)`，RRF 融合，去重 |

## 前端信息架构

侧边栏导航七域：Dashboard / Memory / Knowledge / Wiki / CodeGraph / Jobs / Settings。详见 [../plans/agent-memory-platform/topics/frontend.md](../plans/agent-memory-platform/topics/frontend.md)。
