# AGENTS.md — agent-memory 操作契约

单用户 AI 长期记忆平台：HTTP API 第一公民（AI 操纵平台），Web UI 管理端，Docker 交付。
当前状态：**v0.1.0，八阶段路线全部完成（2026-08-20），独立完成审计批准**。
规划树（权威）：`docs/plantree/`，路线图：`docs/plantree/plans/agent-memory-platform/roadmap.md`。

## 常用命令

```bash
# 后端（在 server/ 下）
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # 集成测试需要 Docker（testcontainers 真 PG）
cargo audit                       # 依赖漏洞基线（见 AGENTS.md「已知开放项」）

# 前端（在 web/ 下）
npm run lint                      # oxlint
npx tsc --noEmit
npm run test                      # vitest（组件测试）
npm run build                     # 含 tsc -b + vite build
npx playwright test               # e2e（需先起栈；chromium: npx playwright install）

# 全栈（在 deploy/ 下；先 cp .env.example .env 并修改）
docker compose up -d --build
curl http://localhost:8080/ready
docker compose down

# OpenAPI 类型再生成（后端 API 变更后必须执行，保持 CI api-types job 绿）
cargo run -p agent-memory-api --bin openapi-dump > /tmp/openapi.json
npx openapi-typescript /tmp/openapi.json -o src/lib/api-schema.ts
```

## 模块边界

实际依赖方向（以各 crate 的 Cargo.toml 为准，详见
`docs/plantree/baseline/module-map.md`）：

```text
api → (storage, jobs, llm, core, distill)
core → (jobs, llm, search, distill, parsing, wiki-engine, cg-bridge)
wiki-engine → (jobs, llm, search, distill, parsing)
parsing → (无内部依赖)
storage → (sqlx only：pool 装配 + 迁移执行)
```

- 域逻辑统一放 `core`：memory/knowledge 服务 + wiki/codegraph 门面（`core::wiki` /
  `core::codegraph` re-export），api 只经 core 访问域服务（Q9 已收敛，2026-08）。
- 文档解析在独立底层 `parsing` crate（core 与 wiki-engine 共用，解开原依赖环）。
- `storage` 是 pool+迁移薄层，查询直接用 sqlx 写在 `core`/`api`（未建仓储抽象层）。
- `llm`/`jobs`/`parsing` 不依赖任何内部 crate。
- 禁止域 crate 之间横向 import（如 `distill` → `wiki-engine`）；跨域编排放 `core`。
- schema 只能改 `server/migrations/`（新增迁移文件，不改已应用的历史迁移）。

## 编码约定

- Rust：edition 2024；错误处理用 `thiserror` 定义类型化错误，API 边界统一转 `ApiError`。
  错误体契约 `{"error":{code,message,retryable}}`：code 稳定可编程判断、message 不泄漏
  内部细节、内部细节只进日志（见 `server/crates/api/src/error.rs`）。
- 日志用 `tracing`（结构化字段，禁止 println）。
- sqlx：运行时 API（`query` / `query_as`），不用编译期 `query!` 宏——避免构建依赖 DATABASE_URL。
- 提示词模板：代码内常量 + `PROMPT_VERSION`，修改必须升版本。
- 前端：类型从 OpenAPI 生成到 `web/src/lib/api-schema.ts`（`npm run gen:api` 或上文的
  openapi-dump 流程），禁止手写重复后端类型；服务端状态走 TanStack Query，
  UI 状态用 React 本地 state（zustand 曾声明于 D0006，落地未使用，2026-08 移除，见 Q10）。
- 所有时间 ISO8601 UTC；ID 用 uuid v7。

## 质量门禁

- 通用：fmt/clippy/test 全绿、compose 可起、文档同步、证据留档（历史阶段出口标准见
  `docs/plantree/plans/agent-memory-platform/phases/`）。
- CI（`.github/workflows/`）：`ci.yml` 四 job（backend fmt/clippy/test、web lint/tsc/
  vitest/build、api-types OpenAPI 漂移检查、docker build）；`e2e.yml` 起 compose 栈跑
  playwright 全旅程。

## 已知开放项（初始化审计 2026-08 记录，详见 open-questions.md）

- **Q8（lopdf 高危）已解决 2026-08-23**：pdf-extract 0.8→0.12（lopdf ≥0.42）。
  余下低风险：tokio-tar（仅 testcontainers dev）、rsa（lockfile 孤儿）、
  ttf-parser unmaintained 警告（pdf-extract 0.12 传递依赖）。
- **Q9（api→core 边界）已解决 2026-08-23**：parsing crate 解环 + core 门面（见上）。
- **Q10（zustand）已解决 2026-08-23**：依赖移除 + D0006 注记。
- **Q11（test-results/）已解决 2026-08-20**：gitignore + 移出跟踪。
