# Evidence — agent-memory-platform

按阶段归档验证证据。每条证据 = 何时、验证了什么、命令/输出摘要、结论。

## Phase 1 — 核心底座（2026-10 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| 全量迁移干净 PG 可重放 | ✅ | `migrations_apply_on_clean_pgvector`：10 份迁移应用 + 幂等重放；`all_domain_tables_exist_with_columns`：16 表存在、vector(1024) 列型、状态机 CHECK、幂等键唯一约束全部验证 |
| fake job 全生命周期 | ✅ | jobs 集成测试 5 项：成功路径（入队→抢占→完成→事件）/重试退避→dead/永久失败直接 failed/dead 复活/幂等键去重/僵尸回收/Runner 真执行（double x21→42） |
| 真实 provider 连通 + embedding 记账 | ✅ | `scripts/verify-real-provider.sh`（对真网关）：chat 1173ms + embed 480ms×4096 维；llm_usage 记账 2 行（4+13 tokens）；密钥密文落库（明文不在 DB） |
| API key 401/403 行为 | ✅ | `auth_401_403_matrix`：无凭证/坏 key/坏会话→401；错密码→401；管理员全通；API key 读 jobs 通、settings/usage→403；完整 key 不回显 |
| OpenAPI 快照 | ✅ | `openapi_snapshot`：13 端点集合快照，变更必须显式更新 |
| mock provider 单测 | ✅ | crypto（加解密往返/错主密钥拒绝）×3 + 路由表 roundtrip + HTTP 错误分类（连接拒绝→瞬态） |
| compose 起栈（Phase 0 标准保持） | ✅ | Phase 1 代码重建镜像→起栈：/ready 200、/auth/login 颁发 token、/jobs 无凭证 401 |

### 关键实现事实

- Runner 优雅停机：watch channel + select 空转轮询可中断
- test_provider 双探测：chat + embedding（有对应能力模型时各发一次探测，均记账）
- EmbedRequest.dimensions（matryoshka 降维）：网关 Qwen3-Embedding-8B 原生 4096 维，
  传 dimensions=1024 适配存储层 vector(1024)（D0010 修订：默认 embedding 通道 = Qwen3-Embedding-8B@1024）
- 真网关：https://newapi.trtyr.top（用户 newapi，key 不落仓库，仅运行时注入）

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
