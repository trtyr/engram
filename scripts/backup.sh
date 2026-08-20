#!/usr/bin/env bash
# agent-memory 备份/恢复。
# 备份:  ./backup.sh backup [输出目录]     → agent-memory-<date>.tar.gz（pg_dump + data 卷）
# 恢复:  ./backup.sh restore <备份文件>    → 重建 db + 数据卷（栈须已停止）
set -euo pipefail

COMPOSE_FILE="$(cd "$(dirname "$0")/../deploy" && pwd)/docker-compose.yml"

cmd="${1:?用法: backup.sh backup|restore}"
case "$cmd" in
  backup)
    out_dir="${2:-.}"
    ts=$(date +%Y%m%d-%H%M%S)
    tmp=$(mktemp -d "/tmp/am-backup.XXXXXX")
    echo "== pg_dump"
    docker compose -f "$COMPOSE_FILE" exec -T db \
      pg_dump -U "${POSTGRES_USER:-agent}" -d "${POSTGRES_DB:-agent_memory}" > "$tmp/db.sql"
    echo "== 打包数据卷"
    docker compose -f "$COMPOSE_FILE" exec -T app tar czf - -C /app data > "$tmp/data.tar.gz" 2>/dev/null || \
      docker run --rm -v agent-memory_appdata:/data alpine tar czf - -C / data > "$tmp/data.tar.gz"
    tar czf "$out_dir/agent-memory-$ts.tar.gz" -C "$tmp" db.sql data.tar.gz
    rm -rf "$tmp"
    echo "完成: $out_dir/agent-memory-$ts.tar.gz"
    ;;
  restore)
    f="${2:?需要备份文件路径}"
    tmp=$(mktemp -d "/tmp/am-restore.XXXXXX")
    tar xzf "$f" -C "$tmp"
    echo "== 恢复数据卷"
    docker run --rm -v agent-memory_appdata:/data -v "$tmp/data.tar.gz":/b.tar.gz alpine \
      sh -c "rm -rf /data/* && tar xzf /b.tar.gz -C /data"
    echo "== 恢复数据库"
    docker compose -f "$COMPOSE_FILE" up -d db
    for i in $(seq 1 30); do
      docker compose -f "$COMPOSE_FILE" exec -T db pg_isready -U "${POSTGRES_USER:-agent}" >/dev/null 2>&1 && break
      sleep 1
    done
    # 先起空库（迁移由 app 启动建表），再灌数据
    docker compose -f "$COMPOSE_FILE" exec -T db \
      psql -U "${POSTGRES_USER:-agent}" -d "${POSTGRES_DB:-agent_memory}" -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;' >/dev/null
    docker compose -f "$COMPOSE_FILE" exec -T db \
      psql -U "${POSTGRES_USER:-agent}" -d "${POSTGRES_DB:-agent_memory}" < "$tmp/db.sql" >/dev/null
    echo "== 完成。启动栈: docker compose -f $COMPOSE_FILE up -d"
    rm -rf "$tmp"
    ;;
  *) echo "未知命令: $cmd"; exit 1;;
esac
