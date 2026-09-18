#!/usr/bin/env bash
# engram 备份/恢复（宿主优先，docker 兜底）——公网多Agent P001-t8。
# 宿主模式（推荐，与 engramctl 运行时同源）：直接 pg_dump/psql + 数据根 tar；
#   环境变量覆盖：AGENT_MEMORY_DATABASE_URL / AGENT_MEMORY_DATA_DIR
# Docker 模式（兜底）：宿主无 pg_dump/psql 或数据目录缺失时落回 deploy/ 栈。
# 用法：
#   ./backup.sh backup [输出目录]      → engram-<date>.tar.gz（db.sql + data.tar.gz）
#   ./backup.sh restore <备份文件>     → 重建 db + 数据目录（**服务须已停止**）
set -euo pipefail

COMPOSE_FILE="$(cd "$(dirname "$0")/../deploy" && pwd)/docker-compose.yml"
DSN="${AGENT_MEMORY_DATABASE_URL:-${ENGRAM_DATABASE_URL:-postgres://trtyr@127.0.0.1:5432/engram}}"
DATA_DIR="${AGENT_MEMORY_DATA_DIR:-$HOME/.engram/app}"

have() { command -v "$1" >/dev/null 2>&1; }

dump_db() { # $1 = 输出 sql（--no-owner --no-privileges：跨角色/跨环境恢复，云机角色 ≠ 本机角色也能灌）
  if have pg_dump; then
    pg_dump --no-owner --no-privileges "$DSN" > "$1"
  else
    echo "宿主无 pg_dump，落回 docker compose 栈 ..." >&2
    docker compose -f "$COMPOSE_FILE" exec -T db \
      pg_dump --no-owner --no-privileges -U "${POSTGRES_USER:-agent}" -d "${POSTGRES_DB:-engram}" > "$1"
  fi
}

load_db() { # $1 = db.sql（灌入前 schema 已重置，迁移由 app 启动时补齐到最新）
  if have psql; then
    psql "$DSN" < "$1"
  else
    docker compose -f "$COMPOSE_FILE" exec -T db \
      psql -U "${POSTGRES_USER:-agent}" -d "${POSTGRES_DB:-engram}" < "$1"
  fi
}

reset_schema() {
  if have psql; then
    psql "$DSN" -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;' >/dev/null
  else
    docker compose -f "$COMPOSE_FILE" up -d db >/dev/null
    for i in $(seq 1 30); do
      docker compose -f "$COMPOSE_FILE" exec -T db pg_isready -U "${POSTGRES_USER:-agent}" >/dev/null 2>&1 && break
      sleep 1
    done
    docker compose -f "$COMPOSE_FILE" exec -T db \
      psql -U "${POSTGRES_USER:-agent}" -d "${POSTGRES_DB:-engram}" \
      -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;' >/dev/null
  fi
}

pack_data() { # $1 = 输出 tar.gz
  if [ -d "$DATA_DIR" ]; then
    tar czf "$1" -C "$(dirname "$DATA_DIR")" "$(basename "$DATA_DIR")"
  else
    echo "宿主数据目录不存在（${DATA_DIR}），落回 docker 数据卷 ..." >&2
    docker run --rm -v engram_appdata:/data alpine tar czf - -C / data > "$1"
  fi
}

unpack_data() { # $1 = data.tar.gz
  if [ -d "$(dirname "$DATA_DIR")" ]; then
    rm -rf "$DATA_DIR"
    tar xzf "$1" -C "$(dirname "$DATA_DIR")"
  else
    docker run --rm -v engram_appdata:/data -v "$1":/b.tar.gz alpine \
      sh -c "rm -rf /data/* && tar xzf /b.tar.gz -C /data"
  fi
}

cmd="${1:?用法: backup.sh backup|restore}"
case "$cmd" in
  backup)
    out_dir="${2:-.}"
    ts=$(date +%Y%m%d-%H%M%S)
    tmp=$(mktemp -d "/tmp/am-backup.XXXXXX")
    echo "== pg_dump（${DSN}）"
    dump_db "$tmp/db.sql"
    echo "== 打包数据根（${DATA_DIR}）"
    pack_data "$tmp/data.tar.gz"
    tar czf "$out_dir/engram-$ts.tar.gz" -C "$tmp" db.sql data.tar.gz
    rm -rf "$tmp"
    echo "完成: $out_dir/engram-$ts.tar.gz"
    ;;
  restore)
    f="${2:?需要备份文件路径}"
    tmp=$(mktemp -d "/tmp/am-restore.XXXXXX")
    tar xzf "$f" -C "$tmp"
    echo "== 重置数据库 schema"
    reset_schema
    echo "== 恢复数据库"
    load_db "$tmp/db.sql" >/dev/null
    echo "== 恢复数据根（${DATA_DIR}）"
    unpack_data "$tmp/data.tar.gz"
    echo "== 完成。重启服务: ~/.engram/bin/engramctl restart"
    rm -rf "$tmp"
    ;;
  *) echo "未知命令: $cmd"; exit 1;;
esac
