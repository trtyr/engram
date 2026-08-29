# 架构

> 全栈模块地图。后端 crate 内部细节（每文件职责表）见 [server/docs/architecture.md](../server/docs/architecture.md)——那里仍是权威，本文档记录全仓视角与该文档生成后的增量变化。

## 仓库布局（monorepo）

```text
agent-memory/
├── server/                  # Rust 后端 workspace（详见下节）
│   ├── Cargo.toml           # workspace 定义 + workspace.dependencies
│   ├── migrations/          # schema 唯一定义处（14 个迁移，启动自动执行）
│   ├── crates/              # 10 个 crate（api/core/storage/llm/jobs/distill/
│   │                        #   search/wiki-engine/cg-bridge/parsing）
│   └── docs/                # 后端深度归档：域审计×4、Wiki 专项、plantree 规划树
├── web/                     # React SPA 前端（详见下节）
│   ├── src/                 # App 壳 + features 七域页 + components + lib
│   ├── e2e/                 # Playwright 全旅程 spec（journey.spec.ts）
│   └── docs/                # 前端对接审计（api-alignment-audit.md）
├── deploy/                  # docker-compose.yml + 多阶段 Dockerfile + .env.example
├── scripts/                 # e2e（Python API 测试套件 ×13）+ verify-*.sh ×3 + backup.sh
├── .github/workflows/       # ci.yml（backend/web/api-types/docker）+ e2e.yml（compose Playwright）
└── data/                    # 运行时数据目录（uploads/wiki-sources/codegraph；空、gitignore）
```

## server/ — Rust 后端（10 crate）

分层按「层 + 域」切分，边界规则：

- **api 只经 core 访问域服务**；wiki/codegraph 在 core 里是纯门面（re-export `wiki-engine` / `cg-bridge`）。
- **storage 是 pool + 迁移薄层**，只被 api（main.rs 装配）使用；core 等 crate 拿 `sqlx::PgPool` 直接写 SQL。
- **禁止域 crate 横向 import**（如 distill → wiki-engine）；跨域编排放 core。
- **llm / jobs / search / cg-bridge / parsing / storage 是叶子**（无内部依赖）。

```text
api ─────────→ storage, jobs, llm, core, distill     （装配：连接池/Runner/域服务）
core ────────→ jobs, llm, search, distill, parsing, wiki-engine, cg-bridge
wiki-engine ─→ jobs, llm, search, distill, parsing
distill ─────→ jobs, llm, search
```

### 与 server/docs/architecture.md（2026-08-26）的增量

| 项 | 旧文档 | 现状（2026-08-28 核实） |
|---|---|---|
| 迁移数 | 12 | **14**（新增 0013 `scenarios.hit_count`、0014 `wiki_sources.error`） |
| 路由模块 | 8 个 | **9 个**（新增 `search_api.rs`：`POST /search` 跨域统一检索） |
| LLM 设置端点 | 仅创建/列出/测试 provider | 增加生命周期：`PUT/DELETE /settings/llm/providers/{id}`、`POST /settings/llm/providers/re-encrypt`（主密钥重加密） |
| 知识端点 | — | 新增 `POST /knowledge/documents/{id}/re-embed` |
| 集成测试基建 | testcontainers（需 Docker） | **本机 PG 每测试一库**（`AM_TEST_PG_URL`，默认 `postgres://127.0.0.1:5432/postgres`；各 crate `tests/support/mod.rs`） |

启动装配顺序（`server/crates/api/src/main.rs`）：`Config::from_env` → JSON 日志 → 连接池 → 迁移 → 注册 job handler（distill 链 → 知识摄取 → wiki 摄取）→ `AppState` → 路由（public + authed + SPA 兜底）→ 监听 `0.0.0.0:8080` → 优雅停机。

## web/ — React 前端

```text
web/src/
├── main.tsx / App.tsx        # 入口 + 应用壳：登录守卫（/jobs?limit=1 探活）+ 侧边栏七域导航
├── features/                 # 每域一页（与后端域一一对应）
│   ├── Login.tsx             #   登录（admin 密码 → ams_ token）
│   ├── Dashboard.tsx         #   总览
│   ├── Memory.tsx            #   L0~L3 会话/原子/场景/画像 + 蒸馏触发（504 行，最大页）
│   ├── Knowledge.tsx         #   文档上传/URL 摄取/检索
│   ├── Wiki.tsx              #   页面/原料/检索/purpose + 子组件（图、洞察、review 队列）
│   ├── CodeGraph.tsx         #   项目注册/索引/查询
│   ├── Jobs.tsx              #   任务列表/事件流/复活
│   ├── Settings.tsx          #   LLM provider/routing/API key/用量
│   └── *.test.tsx            #   vitest 组件测试（与页面同目录共置）
├── components/
│   ├── ui/                   #   shadcn 风格基础组件（button 等）
│   ├── ui-bits.tsx           #   品牌组件（BrandMark 等）
│   ├── WikiGraph.tsx         #   sigma.js 链接图渲染（graphology 数据结构）
│   ├── WikiMarkdown.tsx      #   Markdown 渲染（react-markdown + mermaid）
│   ├── InsightsPanel.tsx     #   Wiki 洞察面板
│   └── ReviewQueue.tsx       #   Wiki review 队列
└── lib/
    ├── api.ts                #   API client：fetch 封装 + token 管理 + 域类型（250 行）
    ├── api-schema.ts         #   OpenAPI 生成的类型（2941 行，pnpm run gen:api 再生成）
    ├── ui.ts / utils.ts      #   UI 工具（cn 等）
```

状态管理：`@tanstack/react-query`（服务端状态）+ 少量本地 state；路由 `react-router-dom` v7；样式 Tailwind 4 + `tw-animate-css`，UI 件 radix-ui。

## 前后端边界

- 开发：Vite dev server 代理 9 个前缀（`/api /auth /jobs /memory /knowledge /wiki /codegraph /settings /llm`）到 `VITE_PROXY_TARGET`（默认 `http://localhost:8080`）。
- 生产：`web/dist` 经 rust-embed 内嵌进后端二进制，非 API 路径兜底到 SPA（`api/src/web_assets.rs`，`routes/mod.rs` 的 `fallback_service`）——单端口、同源、无 CORS。
- 类型契约：OpenAPI（55 paths）→ `openapi-typescript` → `src/lib/api-schema.ts`；CI `api-types` job 做漂移检查。详见 [frontend-backend.md](frontend-backend.md)。

## scripts/ — 验证与运维

| 路径 | 作用 |
|---|---|
| `scripts/e2e/*.py`（13 个）+ `run_all.py` | Python API 级 e2e：health/auth/apikeys/jobs/knowledge/memory/search/wiki/codegraph/llm（`_lib/` 共享 client） |
| `scripts/verify-ai-loop.sh` | AI 视角闭环：仅持 API key 走完记忆→蒸馏→检索全流程（需真 LLM） |
| `scripts/verify-memory-e2e.sh` | 记忆端到端验证 |
| `scripts/verify-real-provider.sh` | 真实 provider 连通 + embedding 记账 |
| `scripts/backup.sh` | pg_dump + data 卷 备份/恢复（对 compose 栈） |
