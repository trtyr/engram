#!/usr/bin/env bash
# 从本地 brew PG（am_dev）迁移数据到 Docker 容器 PG（~/.engram 部署）。
# 用法：deploy/migrate-from-local.sh
# 前置：docker compose up db 已在跑；本机 pg_dump 可用（postgresql@16）。
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DEPLOY_DIR"
ENV_FILE="${ENV_FILE:-$HOME/.engram/.env}"
# shellcheck disable=SC1090
set -a; source "$ENV_FILE"; set +a

LOCAL_DB_URL="${LOCAL_DB_URL:-postgres://127.0.0.1:5432/am_dev}"
BACKUP_DIR="$HOME/.engram/backups"
STAMP="$(date +%Y%m%d-%H%M%S)"
DUMP="$BACKUP_DIR/am_dev-$STAMP.dump"
PG_DUMP="${PG_DUMP:-/opt/homebrew/opt/postgresql@16/bin/pg_dump}"

CONTAINER="$(docker compose --env-file "$ENV_FILE" ps -q db)"
[ -n "$CONTAINER" ] || { echo "✗ db 容器未运行（先 docker compose up -d db）"; exit 1; }

mkdir -p "$BACKUP_DIR"

echo "① dump 本地 am_dev → $DUMP"
"$PG_DUMP" "$LOCAL_DB_URL" -Fc -f "$DUMP"
du -h "$DUMP" | awk '{print "   大小: " $1}'

echo "② 容器库清空重建（全新容器库，仅丢弃迁移 39 建的空 schema）"
docker exec "$CONTAINER" psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -q <<'SQL'
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
SQL

echo "③ restore 进容器（约束 already exists 类告警属良性，行数对照为准）"
docker exec -i "$CONTAINER" pg_restore \
  -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
  --no-owner --no-privileges < "$DUMP" || echo "⚠ restore 有告警，以下方行数对照为准"

echo "④ 行数对照"
for t in project_docs credentials wiki_pages wiki_page_versions wiki_sources todos llm_providers cg_projects; do
  local_n=$(psql "$LOCAL_DB_URL" -t -A -c "SELECT count(*) FROM $t" 2>/dev/null || echo "表不存在")
  docker_n=$(docker exec "$CONTAINER" psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -t -A -c "SELECT count(*) FROM $t" 2>/dev/null || echo "表不存在")
  flag="✅"; [ "$local_n" = "$docker_n" ] || flag="❌"
  printf '   %s %-24s 本地 %-6s 容器 %s\n' "$flag" "$t" "$local_n" "$docker_n"
done

echo "⑤ 重启 app 使其重新连接已恢复的数据"
docker compose --env-file "$ENV_FILE" restart app >/dev/null
sleep 8
curl -s --max-time 5 "http://localhost:${AGENT_MEMORY_PORT:-8080}/ready" || echo "（app 未就绪，稍后手动 curl /ready）"
echo
echo "迁移完成。回滚：停容器栈（docker compose down），恢复本地服务即可——brew PG 原库未动。"
