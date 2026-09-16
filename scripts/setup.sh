#!/usr/bin/env bash
# setup.sh — engram 冷启动部署引导（macOS / Homebrew 优先）。
#
# 从 GitHub clone 下来、什么环境都没有的机器上，跑这个脚本做三件事：
#   1. 体检全部前置依赖，缺什么给出**可复制**的安装命令（能自动建的不动手）
#   2. 初始化数据层：PG 建库 engram + pgvector 扩展
#   3. 生成 ~/.engram/.env（随机管理员密码 + 随机主密钥；已存在则不动）
# 然后提示两条收尾命令（构建前端 + engramctl start）。
#
# 幂等：重复运行安全——已有的东西一律跳过，不覆盖不重装。
# 注意：本脚本不执行 cargo build / 启动服务，那是 engramctl start 的职责。

set -u
cd "$(dirname "$0")/.." # 仓库根

ok() { printf '  ✅ %s\n' "$1"; }
warn() { printf '  ⚠️  %s\n' "$1"; }
fail() { printf '  ❌ %s\n' "$1"; }
cmd_exists() { command -v "$1" >/dev/null 2>&1; }
BREW=${BREW:-/opt/homebrew/bin/brew}

MISSING=0 # 有硬性缺失即 1（收尾提示不再建议直接 start）

step() { printf '\n== %s ==\n' "$1"; }

echo "🧠 engram 冷启动引导（macOS / Homebrew）"
echo "   仓库根：$(pwd)"
echo "   本脚本只体检与初始化；构建并启动服务用收尾提示的 engramctl 命令。"

# ---------- [1/6] 基础工具 ----------
step "[1/6] 基础工具（git / openssl / python3）"
for t in git openssl python3; do
  if cmd_exists "$t"; then ok "$t"; else
    fail "$t 缺失——macOS 先装命令行工具：xcode-select --install"
    MISSING=1
  fi
done

# ---------- [2/6] Rust（cargo） ----------
step "[2/6] Rust 工具链（cargo）"
if cmd_exists cargo; then
  ok "cargo $(cargo --version | awk '{print $2}')"
else
  warn "cargo 缺失——后端无法构建。安装（约 5 分钟）："
  echo "     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "     装完重开终端（或 source \"\$HOME/.cargo/env\"）后再跑本脚本"
  MISSING=1
fi

# ---------- [3/6] Node + pnpm ----------
step "[3/6] Node + pnpm（前端构建）"
if cmd_exists node; then
  ok "node $(node --version)"
else
  warn "node 缺失——brew install node"
  MISSING=1
fi
if cmd_exists pnpm; then
  ok "pnpm $(pnpm --version)"
else
  warn "pnpm 缺失——npm install -g pnpm（或 brew install pnpm）"
  MISSING=1
fi

# ---------- [4/6] PostgreSQL + pgvector ----------
step "[4/6] PostgreSQL + pgvector（数据层）"
PSQL=""
for cand in psql /opt/homebrew/opt/postgresql@16/bin/psql /opt/homebrew/opt/postgresql@17/bin/psql /usr/local/opt/postgresql@16/bin/psql; do
  if [ -x "$(command -v "$cand")" ]; then PSQL="$cand"; break; fi
done
if [ -z "$PSQL" ]; then
  warn "psql 不可用——推荐：brew install postgresql@17 pgvector && brew services start postgresql@17"
  echo "     （brew 的 pgvector formula 只支持 PG17+；PG16 需源码编译 pgvector，不推荐新手）"
  MISSING=1
else
  ok "psql：$PSQL"
  if pg_isready -h 127.0.0.1 -p 5432 >/dev/null 2>&1 || "$PSQL" -h 127.0.0.1 -p 5432 -d postgres -tc 'SELECT 1;' >/dev/null 2>&1; then
    ok "PG 运行中（127.0.0.1:5432）"
    PG_VERSION=$("$PSQL" -h 127.0.0.1 -p 5432 -d postgres -tc 'SHOW server_version;' 2>/dev/null | awk '{print $1}')
    echo "     版本：${PG_VERSION:-未知}"
  else
    warn "PG 未运行——brew services start postgresql@16（或 @17，装了哪个起哪个）"
    MISSING=1
  fi
fi

# ---------- [5/6] 建库 + pgvector 扩展 ----------
step "[5/6] 建库 engram + pgvector 扩展"
DB="${PGDATABASE:-engram}"
DB_URL="postgres://$(whoami)@127.0.0.1:5432/${DB}"
if [ -z "$PSQL" ]; then
  warn "psql 不可用，跳过建库（装好 PG 后重跑本脚本）"
  MISSING=1
elif ! "$PSQL" -h 127.0.0.1 -p 5432 -d postgres -tc 'SELECT 1;' >/dev/null 2>&1; then
  warn "PG 不可达，跳过建库"
  MISSING=1
else
  if "$PSQL" -h 127.0.0.1 -p 5432 -d postgres -tc "SELECT 1 FROM pg_database WHERE datname='$DB'" | grep -q 1; then
    ok "库 $DB 已存在"
  else
    if "$PSQL" -h 127.0.0.1 -p 5432 -d postgres -c "CREATE DATABASE \"$DB\"" >/dev/null 2>&1; then
      ok "已建库 $DB"
    else
      fail "建库失败——检查 PG 权限后重跑本脚本"
      MISSING=1
    fi
  fi
  # pgvector：建扩展（迁移 0001 也会自建，这里提前探——缺了在收尾就指路，不等启动报错）
  if "$PSQL" -h 127.0.0.1 -p 5432 -d "$DB" -c 'CREATE EXTENSION IF NOT EXISTS vector' >/dev/null 2>&1; then
    ok "pgvector 扩展可用"
  else
    warn "pgvector 扩展装不上——brew 的 pgvector 只支持 PG17+："
    echo "     新装路线：brew install postgresql@17 pgvector && brew services start postgresql@17，重跑本脚本"
    echo "     （PG16 现状：需源码编译 pgvector，见 PGVECTOR README）"
    MISSING=1
  fi
fi

# ---------- [6/6] ~/.engram/.env（已存在则不动） ----------
step "[6/6] 运行时配置 ~/.engram/.env"
mkdir -p "$HOME/.engram"
ENV_FILE="$HOME/.engram/.env"
if [ -f "$ENV_FILE" ]; then
  ok "已存在，跳过（绝不覆盖——主密钥换了旧加密数据解不开）"
else
  ADMIN_PW=$(openssl rand -base64 15 | tr '+/' 'Ax')
  MASTER_KEY=$(openssl rand -hex 32)
  cat >"$ENV_FILE" <<ENV
# engram 运行时配置（setup.sh 生成于 $(date '+%Y-%m-%d %H:%M')）
AGENT_MEMORY_ADMIN_PASSWORD=$ADMIN_PW
AGENT_MEMORY_MASTER_KEY=$MASTER_KEY
ENGRAM_DATABASE_URL=postgres://$(whoami)@127.0.0.1:5432/$DB
RUST_LOG=info
ENV
  chmod 600 "$ENV_FILE"
  ok "已生成 $ENV_FILE（权限 600）"
  echo "     ┌──────────────────────────────────────────┐"
  echo "     │ Web 登录密码（只显示这一次，请保存）：      │"
  echo "     │   $ADMIN_PW  │"
  echo "     └──────────────────────────────────────────┘"
fi

# ---------- 收尾 ----------
echo
echo "== 体检结果 =="
if [ "$MISSING" = "1" ]; then
  echo "有缺失项（见上方 ❌/⚠️）——按提示补齐后**重跑本脚本**确认全绿，再做收尾。"
  echo "收尾（全部就绪后）："
else
  echo "环境就绪 ✅ 收尾两步："
fi
echo "  1) cd web && pnpm install --frozen-lockfile && pnpm build   # 前端产物（rust-embed 编译期需要）"
echo "  2) python3 scripts/engramctl.py start                        # 构建后端 → 安装 → 挂起 → http://localhost:17654"
exit 0
