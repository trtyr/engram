# 运行与部署（全栈）

> 两端的完整命令各自有档案（[server](../server/docs/run-and-deploy.md)、
> [web](../web/docs/run-and-deploy.md)）。本文是最短路径。

## 本地全栈（裸栈，开发期推荐）

```bash
# 1) 库（一次性）
psql -c "CREATE DATABASE am_dev"

# 2) 后端 :8080
cd server
AGENT_MEMORY_DATABASE_URL='postgres://127.0.0.1:5432/am_dev' \
AGENT_MEMORY_ADMIN_PASSWORD='dev-pw' \
AGENT_MEMORY_MASTER_KEY="$(printf 'ab%.0s' {1..32})" \
AGENT_MEMORY_DATA_DIR=/tmp/am-data \
cargo run -q -p agent-memory-api --bin agent-memory-server

# 3) 前端二选一
cd web && pnpm dev                 # 开发模式（代理到 :8080）
pnpm run build                     # 或构建 dist 让 rust-embed 托管（同端口直出）
```

踩坑提示：`--bin agent-memory-server` 不能省（api crate 双二进制）；
MASTER_KEY 必须 64 位 hex；开发期不碰 docker（用户既定方针，CI 负责镜像验证）。

## 全量验证（当日全绿）

```bash
cd server && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build
cd web && E2E_ADMIN_PW=… pnpm exec playwright test   # 需运行中栈
```

## 部署（未来）

deploy/Dockerfile 多阶段（web-build pnpm → cargo-chef → 运行时含 Node+git+codegraph CLI）+
docker-compose.yml。当前阶段仅由 CI docker job 验证可构建，未实际部署。
备份：scripts/backup.sh（pg_dump + 数据卷）。
