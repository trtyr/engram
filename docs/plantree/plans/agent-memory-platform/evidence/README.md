# Evidence — agent-memory-platform

按阶段归档验证证据。每条证据 = 何时、验证了什么、命令/输出摘要、结论。

## Phase 0 — 项目地基（2026-10 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| cargo fmt --check | ✅ 0 diff | 本地 rustc 1.97.1 |
| cargo clippy --workspace --all-targets -D warnings | ✅ 0 警告 | 同上 |
| cargo test --workspace | ✅ 1 passed | `migrations_apply_on_clean_pgvector`：testcontainers 起 pgvector/pgvector:pg17 → 迁移应用 → `SELECT '[1,2,3]'::vector` 成功 → 幂等重放成功 |
| web tsc --noEmit | ✅ 0 错误 | TS 6.x |
| web oxlint | ✅ 仅 shadcn 生成代码已知 fast-refresh 警告 | |
| web npm run build | ✅ 通过 | Vite 7 + Tailwind v4 + shadcn(radix) |
| docker compose build | ✅ 镜像构建成功 | cargo-chef 分层缓存；首次依赖编译约 4 分钟 |
| docker compose up + 探针 | ✅ 全部 200 | `/health`→`{"status":"ok"}`；`/ready`→`{"status":"ready","migration_version":1}`（容器内自动迁移）；`/openapi.json` 正常；未知路由统一错误体 404 |

### 环境备注（本机）

- 本机 8080 与 18080 先后被其他项目占用，本地验证固定用 `AGENT_MEMORY_PORT=19180`（起栈前先 `lsof` 确认空闲）
- Docker daemon：OrbStack；buildx 首次解析基础镜像偶发 auth.docker.io 超时
  → 解法：`docker pull node:22-slim lukemathwalker/cargo-chef:latest-rust-1.97-slim debian:bookworm-slim` 预拉后重建
- CI（GitHub Actions ubuntu-latest）不受上述本机问题影响
