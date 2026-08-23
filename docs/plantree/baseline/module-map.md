# Module Map — 实际模块地图

> 现状：八阶段全部落地（v0.1.0）。2026-08 Q9 收敛后，api 仅经 core 访问域服务
> （wiki/codegraph 门面在 core，底层解析抽到独立 parsing crate）。

## 仓库布局（monorepo）

```
agent-memory/
├── server/                    # Rust 后端（axum 工作区）
│   ├── Cargo.toml             # workspace root
│   ├── crates/
│   │   ├── api/               # HTTP 层：路由、handler、DTO、OpenAPI、中间件（auth/error/log）
│   │   ├── core/              # 领域层：memory/knowledge 服务 + wiki/codegraph 门面 + 任务编排
│   │   ├── storage/           # pool 装配 + 迁移执行（薄层，无仓储抽象）
│   │   ├── llm/               # LLM 客户端：provider 抽象、模型路由、用量统计
│   │   ├── jobs/              # 任务系统：PG-backed 队列、worker、调度、事件
│   │   ├── distill/           # 蒸馏管道：L0→L1→L2→L3 编排、提示词模板
│   │   ├── wiki-engine/       # Wiki 引擎：两步 ingest、wikilink、lint、图数据
│   │   ├── cg-bridge/         # CodeGraph 桥：CLI 子进程包装、JSON 解析、项目注册
│   │   ├── search/            # 混合检索：FTS + 向量 + RRF 融合、预算控制
│   │   └── parsing/           # 文档解析：pdf/docx/html/md/txt → 纯文本（底层，core/wiki-engine 共用）
│   └── migrations/            # sqlx 迁移（唯一定义处）
├── web/                       # React + Vite 前端
│   └── src/
│       ├── features/          # dashboard/memory/knowledge/wiki/codegraph/jobs/settings
│       ├── components/        # 共享 UI（shadcn/ui 定制）
│       └── lib/               # API client、类型（从 OpenAPI 生成）
├── deploy/                    # Dockerfile、docker-compose.yml、.env.example
└── docs/plantree/             # 本计划树
```

## 依赖方向（单向，实际）

以各 crate 的 Cargo.toml 为准（2026-08 初始化审计 + Q9 收敛后核实）：

```text
api → (storage, jobs, llm, core, distill)
core → (jobs, llm, search, distill, parsing, wiki-engine, cg-bridge)
wiki-engine → (jobs, llm, search, distill, parsing)
parsing → (无内部依赖，纯底层)
storage → (sqlx only：pool 装配 + 迁移执行)
```

- 域逻辑统一放 `core`：memory/knowledge 服务 + wiki/codegraph 门面（`core::wiki` /
  `core::codegraph` 平铺 re-export 各域类型与服务，api 只经 core 访问）。
- **Q9 收敛说明**：目标架构曾写 `core → wiki-engine`，但 wiki-engine 反向依赖 core
  （用其 knowledge::parse）形成环。解法：把文档解析抽到独立底层 `parsing` crate，
  wiki-engine 改为依赖 parsing（不再依赖 core），core 依赖 wiki-engine/cg-bridge 并
  以门面暴露给 api——api 不再直连任一域 crate（storage/jobs/llm/distill 为基础设施，
  见 AGENTS.md 边界）。
- `llm`/`jobs`/`parsing` 不依赖任何内部 crate。
- 禁止域间横向 import（`distill` 不 import `wiki-engine`）；跨域协作放 `core` 编排。

## 模块职责一句话

| 模块 | 职责 | 深模块接口要点 |
|---|---|---|
| api | HTTP 契约层 | 路由注册、DTO↔领域转换、统一错误体、OpenAPI |
| core | 领域编排 | memory/knowledge 服务 + wiki/codegraph 门面（re-export）+ 任务触发 |
| storage | 持久化薄层 | pool 装配 + 迁移执行（无仓储 trait，查询在 core/api 直接用 sqlx） |
| llm | 模型访问 | `trait Provider`、路由规则（任务类型→模型）、加密密钥、用量记账 |
| jobs | 异步执行 | job 状态机、重试策略、事件流（SSE 源）、幂等键 |
| distill | 蒸馏编排 | 提示词模板版本化、L0→L3 各阶段任务、consolidation |
| wiki-engine | 知识编译 | 两步 ingest、frontmatter/wikilink 解析、index/log 维护、lint（经 core::wiki 暴露） |
| cg-bridge | 图谱代理 | codegraph CLI 调用、超时/错误归一、项目生命周期（经 core::codegraph 暴露） |
| search | 统一检索 | 单入口 `search(query, scopes, budget)`，RRF 融合，去重 |
| parsing | 文档解析 | pdf/docx/html/md/txt → 纯文本；core 与 wiki-engine 共用（解 Q9 环） |

## 前端信息架构

侧边栏导航七域：Dashboard / Memory / Knowledge / Wiki / CodeGraph / Jobs / Settings。详见 [../plans/agent-memory-platform/topics/frontend.md](../plans/agent-memory-platform/topics/frontend.md)。
