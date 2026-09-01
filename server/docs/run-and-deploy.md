# 运行与部署

> 2026-08-30 实跑验证。开发阶段以本地裸栈为主（用户明确：开发期不关心 docker compose）。

## 前置

- Rust（本机 1.97.1；CI stable）
- PostgreSQL 带 pgvector 扩展（本地 Homebrew 16.14 @127.0.0.1:5432 trust；
  无 pgvector 可 `brew install pgvector`）
- 前端产物：`web/dist` 需存在（rust-embed 编译期检查；CI 里后端 job 有 `mkdir -p ../web/dist` 占位步骤）

## 环境变量（crates/api/src/config.rs）

| 变量 | 必填 | 说明 |
|---|---|---|
| AGENT_MEMORY_DATABASE_URL | ✓ | postgres://…（库需可 CREATE EXTENSION vector） |
| AGENT_MEMORY_ADMIN_PASSWORD | ✓ | 管理员登录密码 |
| AGENT_MEMORY_MASTER_KEY | ✓ | 64 位 hex（provider 密钥加密主密钥；`openssl rand -hex 32`） |
| AGENT_MEMORY_PORT | 默认 8080 | 监听端口 |
| AGENT_MEMORY_DATA_DIR | ✓ | 运行数据目录 |
| RUST_LOG | 可选 | 日志级别 |
| AM_TEST_PG_URL | 测试 | 集成测试用的 PG 连接串 |

## 本地起栈（实测命令）

```bash
# 建库（一次性）
psql -c "CREATE DATABASE am_dev"

# 起服务（注意 --bin：api crate 有两个二进制）
cd server
AGENT_MEMORY_DATABASE_URL='postgres://127.0.0.1:5432/am_dev' \
AGENT_MEMORY_PORT=8080 \
AGENT_MEMORY_ADMIN_PASSWORD='dev-pw' \
AGENT_MEMORY_MASTER_KEY="$(printf 'ab%.0s' {1..32})" \
AGENT_MEMORY_DATA_DIR=/tmp/am-data \
RUST_LOG=info \
cargo run -q -p agent-memory-api --bin agent-memory-server
# 就绪探针：curl :8080/ready（首次含编译 + 迁移）
```

开发模式（debug）下 rust-embed 直读磁盘——`web/dist` 重新构建后无需重启服务。

## 测试（本机 PG，无 Docker）

```bash
cd server && cargo test --workspace   # 141 passed / 0 failed
```

每个测试 crate 经 `tests/support/mod.rs` 用 sqlx 建一次性库、Drop 时
`DROP DATABASE WITH FORCE`；无 AM_TEST_PG_URL 时默认 127.0.0.1:5432。

## OpenAPI 导出

```bash
cargo run -q -p agent-memory-api --bin openapi-dump > openapi.json
```

## 部署（Docker，当前未启用）

`deploy/Dockerfile` 多阶段：web-build（pnpm）→ chef 缓存 → server 构建 → 运行时（含 Node，
codegraph CLI `@colbymchenry/codegraph@1.5.0` + git）。`deploy/docker-compose.yml` 为完整栈。
开发期不维护本地 compose（CI docker job 负责构建验证）。

## 备份

`scripts/backup.sh`：pg_dump + 数据卷备份/恢复。
