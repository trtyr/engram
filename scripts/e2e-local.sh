#!/usr/bin/env bash
# e2e 一次性栈（P11 落地）：独立 DB + 独立端口，跑完自动拆。
# 三次生产库事故后，本地 journey 唯一合法入口。
# 注意：macOS bash 3.2 非多字节感知——变量后紧跟全角字符必须 ${VAR} 写法。
set -euo pipefail
cd "$(dirname "$0")/../web"

DB="am_e2e_$(date +%s)"
PORT="${E2E_PORT:-19199}"
PW="${E2E_ADMIN_PW:-e2e-local-pw}"
ROOT="$(cd .. && pwd)"
LOG="/tmp/e2e-local-$$.log"

echo "▶ 一次性栈: DB=${DB} PORT=${PORT}"
psql -q -d postgres -c "CREATE DATABASE $DB"

cleanup() {
  echo "▶ 拆栈: kill server / drop ${DB}"
  lsof -ti tcp:$PORT | xargs kill 2>/dev/null || true
  sleep 1
  psql -q -d postgres -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$DB' AND pid <> pg_backend_pid()" >/dev/null 2>&1 || true
  psql -q -d postgres -c "DROP DATABASE IF EXISTS $DB" || true
}
trap cleanup EXIT

cd "$ROOT/server"
AGENT_MEMORY_DATABASE_URL="postgres://127.0.0.1:5432/$DB" \
AGENT_MEMORY_PORT=$PORT \
AGENT_MEMORY_ADMIN_PASSWORD=$PW \
AGENT_MEMORY_MASTER_KEY="$(printf 'ab%.0s' {1..32})" \
AGENT_MEMORY_DATA_DIR="$(mktemp -d)" \
RUST_LOG=info \
  nohup cargo run -q -p agent-memory-api --bin agent-memory-server > "$LOG" 2>&1 &

R=""
for i in $(seq 1 90); do
  R=$(curl -s -m 2 "http://127.0.0.1:$PORT/ready" 2>/dev/null || true)
  [ -n "$R" ] && break
  sleep 3
done
if [ -z "$R" ]; then
  echo "✗ 栈未就绪（日志 ${LOG}）："
  tail -5 "$LOG" || true
  exit 1
fi
echo "▶ 就绪: ${R} (日志 ${LOG})"

cd "$ROOT/web"
E2E_BASE="http://127.0.0.1:$PORT" E2E_ADMIN_PW="$PW" pnpm exec playwright test "$@"
