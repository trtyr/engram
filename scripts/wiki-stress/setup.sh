#!/usr/bin/env bash
# wiki 规模化压测环境搭建：独立库 + 临时实例 + 灌数据。
# 用法: ./setup.sh <生成的SQL文件>   例: ./setup.sh stress-1k.sql
# 产物: http://127.0.0.1:17660 压测实例（admin / stress-test-pw）
set -euo pipefail

SQL_FILE="${1:?用法: setup.sh <生成的SQL文件>}"
PORT=17660
STRESS_DB="engram_stress"
STRESS_DIR="/tmp/engram-stress"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SERVER_BIN="$ROOT/server/target/debug/engram-server"
STRESS_DSN="postgres://trtyr@127.0.0.1:5432/$STRESS_DB"

echo "== [1/6] 重建压测库 $STRESS_DB =="
psql "postgres://trtyr@127.0.0.1:5432/postgres" -qc "DROP DATABASE IF EXISTS $STRESS_DB"
psql "postgres://trtyr@127.0.0.1:5432/postgres" -qc "CREATE DATABASE $STRESS_DB"

echo "== [2/6] 清理旧实例与数据目录 =="
OLD_PID=$(lsof -tnP -iTCP:$PORT -sTCP:LISTEN 2>/dev/null || true)
if [ -n "$OLD_PID" ]; then kill "$OLD_PID"; sleep 2; fi
rm -rf "$STRESS_DIR"; mkdir -p "$STRESS_DIR"

echo "== [3/6] 启动临时实例（端口 ${PORT}，迁移自动执行） =="
set -a
# shellcheck disable=SC1091
source "$HOME/.engram/.env" 2>/dev/null || true
set +a
export AGENT_MEMORY_DATABASE_URL="$STRESS_DSN"
export AGENT_MEMORY_DATA_DIR="$STRESS_DIR"
export AGENT_MEMORY_PORT="$PORT"
export AGENT_MEMORY_ADMIN_PASSWORD="stress-test-pw"
unset AGENT_MEMORY_MASTER_KEY
nohup "$SERVER_BIN" > "$STRESS_DIR/server.log" 2>&1 &
echo "server pid: $!"

echo "== [4/6] 等待就绪 =="
for i in $(seq 1 60); do
  if curl -sf -m 2 "http://127.0.0.1:$PORT/ready" >/dev/null 2>&1; then
    echo "就绪（第 ${i} 次探测）"; break
  fi
  sleep 1
  if [ "$i" = "60" ]; then echo "超时——看 $STRESS_DIR/server.log"; exit 1; fi
done
curl -s -m 3 "http://127.0.0.1:$PORT/ready"; echo

echo "== [5/6] 灌入压测数据：$SQL_FILE =="
psql "$STRESS_DSN" -q -f "$SQL_FILE"
PAGES=$(psql "$STRESS_DSN" -tAc "SELECT count(*) FROM wiki_pages")
SRCS=$(psql "$STRESS_DSN" -tAc "SELECT count(*) FROM wiki_sources")
echo "导入完成：pages=$PAGES sources=$SRCS"

echo "== [6/6] 完成 =="
cat <<EOF
压测实例就绪：
  base : http://127.0.0.1:$PORT
  token: $(curl -s -m 5 -X POST "http://127.0.0.1:$PORT/auth/login" -H 'Content-Type: application/json' -d '{"username":"admin","password":"stress-test-pw"}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
  db   : $STRESS_DSN
  停止 : kill \$(lsof -tnP -iTCP:$PORT -sTCP:LISTEN)
EOF
