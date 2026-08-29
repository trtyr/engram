# 运行与部署

> 2026-08-28 实测验证过的命令标注 ✅；已知会失败的标注 ⚠️（原因见 [current-state.md](current-state.md)）。
> 后端细节另见 [server/docs/run-and-deploy.md](../server/docs/run-and-deploy.md)。

## 前置条件

| 项 | 后端开发 | 前端开发 | 部署 |
|---|---|---|---|
| Rust 1.97（含 rustfmt/clippy） | ✅ 必需 | — | 镜像内构建 |
| Node 22 + **pnpm** | — | ✅ 必需 | 镜像内构建 |
| PostgreSQL（本机 5432，trust 认证） | ✅ 集成测试必需（或 `AM_TEST_PG_URL` 覆盖）；跑服务另需一个库 | — | compose 的 `pgvector/pgvector:pg17` |
| Docker | 可选（部署） | 可选 | ✅ 必需 |

## 后端本地开发（server/）

```bash
cd server

cargo fmt --check                                          # ✅
cargo clippy --workspace --all-targets -- -D warnings      # ✅
cargo test --workspace                                     # ✅ 100 passed / 0 failed（本机 PG，约 3.5 分钟）

# 直跑服务（启动即自动迁移，监听 0.0.0.0:8080）
export AGENT_MEMORY_DATABASE_URL="postgres://agent:password@localhost:5432/agent_memory"
cargo run -p agent-memory-api --bin agent-memory-server

# 提取 OpenAPI（不起服务；需 ../web/dist 存在——rust-embed 编译期可见，没有就 mkdir -p ../web/dist）
cargo run -q -p agent-memory-api --bin openapi-dump > /tmp/openapi.json   # ✅ 55 paths
```

集成测试基建：复用本机 Homebrew PG，每测试建独立库 `am_test_<nanos>_<seq>`（template0），Drop 时后台 `DROP DATABASE ... WITH (FORCE)`；`AM_TEST_PG_URL` 覆盖管理库连接串（默认 `postgres://127.0.0.1:5432/postgres`）。

## 前端本地开发（web/）

```bash
cd web

pnpm install --frozen-lockfile     # ✅（注意：是 pnpm，不是 npm——没有 package-lock.json）
pnpm run lint                      # ✅ oxlint（裸 `pnpm lint` 偶发解析怪癖，用 run 形式稳）
pnpm test                          # ✅ vitest：4 文件 21 测试全过（约 23 秒）
pnpm run build                     # ✅ tsc -b + vite build（无 >500kB 警告；初始 bundle ~280kB）

pnpm dev                           # Vite dev server，代理 9 前缀 → localhost:8080

pnpm run gen:api                   # 从 OPENAPI_URL（默认 :8090）拉 openapi.json 再生成 api-schema.ts
```

## 环境变量

后端（`server/crates/api/src/config.rs`，全部 `AGENT_MEMORY_` 前缀）：

| 变量 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `AGENT_MEMORY_DATABASE_URL` | ✅ | — | PostgreSQL 连接串 |
| `AGENT_MEMORY_PORT` | — | 8080 | HTTP 端口 |
| `AGENT_MEMORY_ADMIN_PASSWORD` | — | — | 管理员登录密码（未设仅告警） |
| `AGENT_MEMORY_MASTER_KEY` | — | — | 密钥加密主密钥（`openssl rand -hex 32`；丢失则已存 provider key 无法解密；可换钥后 `POST /settings/llm/providers/re-encrypt`） |
| `AGENT_MEMORY_DATA_DIR` | — | `./data` | 运行时数据（uploads/wiki-sources/codegraph） |
| `RUST_LOG` | — | info | 日志级别 |
| `AM_TEST_PG_URL` | — | `postgres://127.0.0.1:5432/postgres` | 集成测试管理库连接串 |

部署（`deploy/.env`，模板 `.env.example`）：`POSTGRES_USER/PASSWORD/DB`、`AGENT_MEMORY_ADMIN_PASSWORD`、`AGENT_MEMORY_MASTER_KEY`、`AGENT_MEMORY_PORT`。

前端：`VITE_PROXY_TARGET`（dev 代理目标）、`OPENAPI_URL`（gen:api 的 OpenAPI 源）、e2e 的 `E2E_BASE`/`E2E_ADMIN_PW`。

## Docker 部署

```bash
cd deploy
cp .env.example .env && $EDITOR .env    # 改全部 change-me 项
docker compose up -d --build            # Stage1 已改 corepack+pnpm（完整镜像构建由 CI docker job 验证）
curl http://localhost:8080/health       # {"status":"ok"}
```

compose 两服务：`db`（pgvector/pgvector:pg17，pg_isready 健康检查）+ `app`（本地构建镜像，depends_on service_healthy，挂 `/app/data` 卷）。镜像运行时含 Node + git（codegraph CLI pin `@colbymchenry/codegraph@1.5.0`），HEALTHCHECK 打 `/ready`。

> 宿主跑 Clash TUN（fake-ip）时容器 DNS 被污染，SSRF 防护会拒绝 fake-ip 地址——给容器配 `HTTPS_PROXY=http://host.docker.internal:7892` 可恢复外网摄取（compose 注释原文）。

首次配置：登录 → Settings → providers 注册 LLM provider →（可选）routing 配 purpose 路由。

### 备份与恢复

```bash
scripts/backup.sh backup              # pg_dump + data 卷 → agent-memory-<date>.tar.gz
scripts/backup.sh restore <file>      # 栈停止后恢复
```

## e2e 与验证脚本

| 命令 | 前置 | 说明 |
|---|---|---|
| `cd web && E2E_BASE=... E2E_ADMIN_PW=... npx playwright test` | 运行中的栈 | 全旅程：上传→ready、会话→蒸馏→原子、wiki→页面+图谱、codegraph→查询（journey.spec.ts，10 分钟超时；默认打 127.0.0.1:19180） |
| `python3 scripts/e2e/run_all.py` | 运行中的栈 | 13 个 Python API e2e 顺序跑（health/auth/apikeys/jobs/knowledge/memory/search/wiki/codegraph/llm） |
| `scripts/verify-real-provider.sh <BASE> <KEY> <CHAT> <EMBED>` | 运行中的栈 + 真 key | provider 连通 + embedding 记账 |
| `scripts/verify-ai-loop.sh <BASE> <ADMIN_PW>` | 干净库 + 真 provider | AI 视角闭环：仅持 API key 走完记忆→蒸馏→检索 |
| `scripts/verify-memory-e2e.sh` | 运行中的栈 | 记忆端到端 |

CI（`.github/workflows/e2e.yml`）在 push main/PR 时起 compose 栈跑 Playwright 全旅程——无 LLM provider 时蒸馏/Wiki 生成断言自动跳过（annotation 留痕），其余旅程照常。
