# AGENTS.md — agent-memory 操作契约

单用户 AI 长期记忆平台：HTTP API 第一公民（AI 操纵平台），Web UI 管理端，Docker 交付。
规划树（权威）：`docs/plantree/`，路线图：`docs/plantree/plans/agent-memory-platform/roadmap.md`。

## 常用命令

```bash
# 后端（在 server/ 下）
cargo fmt && cargo clippy --workspace --all-targets -D warnings
cargo test --workspace            # 集成测试需要 Docker（testcontainers）

# 前端（在 web/ 下）
npm run build                     # 含 tsc -b
npx tsc --noEmit

# 全栈（在 deploy/ 下；先 cp .env.example .env 并修改）
docker compose up -d --build
curl http://localhost:8080/ready
docker compose down
```

## 模块边界（禁止违反）

依赖方向（`docs/plantree/baseline/module-map.md`）：

```
api → core → (storage, llm, jobs, search, distill, wiki-engine, cg-bridge)
```

- `api` 不直接 import `storage` 之外的 crate 进行数据访问——新增域逻辑一律放 `core`（Phase 1 起）。
- `storage` 不依赖任何内部 crate；`llm`/`jobs` 同理保持底层独立。
- 禁止域 crate 之间横向 import（如 `distill` → `wiki-engine`）；跨域编排放 `core`。
- schema 只能改 `server/migrations/`（新增迁移文件，不改已应用的历史迁移）。

## 编码约定

- Rust：edition 2024；错误处理用 `thiserror` 定义类型化错误，API 边界统一转 `ApiError`；
  日志用 `tracing`（结构化字段，禁止 println）。
- sqlx：用运行时 API（`query` / `query_as`），不用编译期 `query!` 宏——避免构建依赖 DATABASE_URL。
- 提示词模板：代码内常量 + `PROMPT_VERSION`，修改必须升版本。
- 前端：类型从 OpenAPI 生成（Phase 1 起接入 `lib/api-types.ts`），禁止手写重复后端类型；
  服务端状态走 TanStack Query，UI 状态走 Zustand。
- 所有时间 ISO8601 UTC；ID 用 uuid v7。

## 阶段出口门禁

每阶段完成定义见 `docs/plantree/plans/agent-memory-platform/phases/phase-N-*.md` 的
「出口标准」。通用要求：fmt/clippy/test 全绿、compose 可起、文档同步、证据留档。
