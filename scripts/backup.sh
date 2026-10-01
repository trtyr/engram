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
# 注意：fallback DSN 的角色（trtyr）与库名（engram）是本机专属值——换机器请用
# AGENT_MEMORY_DATABASE_URL 或 ENGRAM_DATABASE_URL 覆盖，勿改此默认值。
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
    echo "错误：宿主数据目录不存在（${DATA_DIR}），无数据可打包。" >&2
    echo "检查 AGENT_MEMORY_DATA_DIR；拒绝产出空备份（P001-T001：不再落回已弃用的 named volume）。" >&2
    exit 1
  fi
}

unpack_data() { # $1 = data.tar.gz
  mkdir -p "$(dirname "$DATA_DIR")"
  local staging="${DATA_DIR}.restore-tmp"
  rm -rf "$staging" && mkdir -p "$staging"
  # P003-T002 原子交换：先解到暂存区，解包成功才替换活数据——档损坏时活数据不被先删
  if ! tar xzf "$1" -C "$staging"; then
    rm -rf "$staging"
    echo "错误：备份档解包失败，活数据未动（P003-T002 原子交换）。" >&2
    exit 1
  fi
  rm -rf "$DATA_DIR"
  mv "$staging"/* "$DATA_DIR"/
  rmdir "$staging" 2>/dev/null || true
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
