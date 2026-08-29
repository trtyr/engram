# 运行与部署

## 本地开发

后端是 Rust workspace（`server/`），集成测试用**本机 PostgreSQL**（每测试建/删独立库 `am_test_*`，默认连 `postgres://127.0.0.1:5432/postgres`，`AM_TEST_PG_URL` 可覆盖），不再需要 Docker。

```bash
cd server

# 构建
cargo build

# 格式检查
cargo fmt --check

# lint（警告即错误）
cargo clippy --workspace --all-targets -- -D warnings

# 测试（需本机 PG 可达）
cargo test --workspace

# 依赖漏洞扫描（基线）
cargo audit
```

> 命令顺序与 CI `backend` job 一致（fmt → clippy → test）。

### 本地直跑（不起 Docker compose）

```bash
cd server
export AGENT_MEMORY_DATABASE_URL="postgres://agent:password@localhost:5432/agent_memory"
cargo run -p agent-memory-api   # 监听 0.0.0.0:8080，启动即自动迁移
```

### 提取 OpenAPI（不起服务）

```bash
cd server
cargo run -p agent-memory-api --bin openapi-dump > /tmp/openapi.json
```

（前端类型由 `openapi-typescript` 从这份 json 生成——前端跑 `pnpm run gen:api`；CI 的 `api-types` job 用它做漂移检查。）

## 环境变量

全部带 `AGENT_MEMORY_` 前缀（`server/crates/api/src/config.rs`）：

| 变量 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `AGENT_MEMORY_DATABASE_URL` | ✅ | — | PostgreSQL 连接串 |
| `AGENT_MEMORY_PORT` | — | 8080 | HTTP 监听端口 |
| `AGENT_MEMORY_ADMIN_PASSWORD` | — | — | 管理员密码（Web UI 登录；未设仅告警） |
| `AGENT_MEMORY_MASTER_KEY` | — | — | 密钥加密主密钥（32 字节随机 hex，`openssl rand -hex 32`；未设仅告警） |
| `AGENT_MEMORY_DATA_DIR` | — | `./data` | 运行时数据目录（uploads/wiki-sources/codegraph） |
| `RUST_LOG` | — | `info` | 日志级别（`tracing` EnvFilter） |

## Docker 部署

生产交付是单镜像（`deploy/Dockerfile` 多阶段：前端构建 → cargo-chef 后端构建 → debian trixie 运行时）。运行时镜像内含 Node + git（供 codegraph CLI 使用，pin `@colbymchenry/codegraph@1.5.0`）。

```bash
cd deploy
cp .env.example .env
$EDITOR .env          # 改全部 change-me 项
docker compose up -d --build

curl http://localhost:8080/health    # {"status":"ok"}
curl http://localhost:8080/ready     # 就绪探针
docker compose down
```

`docker-compose.yml` 两个服务：

| 服务 | 镜像 | 说明 |
|---|---|---|
| `db` | `pgvector/pgvector:pg17` | PostgreSQL 17 + pgvector，健康检查 `pg_isready` |
| `app` | 本地构建（`deploy/Dockerfile`） | `depends_on: db`（service_healthy），挂载 `/app/data` 卷 |

首次配置（Web UI 登录后）：Settings → providers 注册 LLM provider →（可选）Settings → routing 配 purpose 路由。

### 备份与恢复

```bash
scripts/backup.sh backup              # → agent-memory-<date>.tar.gz
scripts/backup.sh restore <file>      # 栈停止后恢复
```

## 健康检查

| 端点 | 用途 |
|---|---|
| `GET /health` | 存活探针 |
| `GET /ready` | 就绪探针（Dockerfile HEALTHCHECK 与 compose e2e 等待用） |

## 验证脚本

`scripts/` 下提供三套端到端验证（供 CI/本地回归）：

| 脚本 | 作用 |
|---|---|
| `verify-ai-loop.sh` | AI 循环（记忆写入→蒸馏→检索）验证 |
| `verify-memory-e2e.sh` | 记忆端到端验证 |
| `verify-real-provider.sh` | 真实 LLM provider 连通验证 |
