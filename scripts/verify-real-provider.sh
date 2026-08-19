#!/usr/bin/env bash
# Phase 1 出口验证：真实 provider 连通 + embedding 记账。
# 用法: ./scripts/verify-real-provider.sh <BASE_URL> <API_KEY> <CHAT_MODEL> <EMBED_MODEL>
set -euo pipefail

BASE_URL="${1:?需要 base_url}"
API_KEY="${2:?需要 api_key}"
CHAT_MODEL="${3:-}"
EMBED_MODEL="${4:-}"

PORT=19537
PG_PORT=19538
WORKDIR=$(mktemp -d /tmp/am-verify.XXXXXX)
SERVER_DIR="$(cd "$(dirname "$0")/../server" && pwd)"

cleanup() {
  [ -n "${SRV_PID:-}" ] && kill "$SRV_PID" 2>/dev/null || true
  docker rm -f am-verify-pg >/dev/null 2>&1 || true
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

echo "== 1. 起 pgvector"
docker run -d --name am-verify-pg -e POSTGRES_PASSWORD=verify -e POSTGRES_DB=am \
  -p "$PG_PORT:5432" pgvector/pgvector:pg17 >/dev/null
for i in $(seq 1 30); do
  docker exec am-verify-pg pg_isready -U postgres -d am >/dev/null 2>&1 && break
  sleep 1
done

echo "== 2. 起服务"
MASTER_KEY=$(openssl rand -hex 32)
AGENT_MEMORY_DATABASE_URL="postgres://postgres:verify@127.0.0.1:$PG_PORT/am" \
AGENT_MEMORY_PORT=$PORT \
AGENT_MEMORY_ADMIN_PASSWORD=verify-admin \
AGENT_MEMORY_MASTER_KEY="$MASTER_KEY" \
  "$SERVER_DIR/target/debug/agent-memory-server" >"$WORKDIR/server.log" 2>&1 &
SRV_PID=$!
for i in $(seq 1 30); do
  curl -fsS "http://127.0.0.1:$PORT/ready" >/dev/null 2>&1 && break
  sleep 1
done
echo "   ready ✓"

echo "== 3. 登录 + 注册 provider"
TOKEN=$(curl -fsS -X POST "http://127.0.0.1:$PORT/auth/login" \
  -H 'content-type: application/json' -d '{"password":"verify-admin"}' | jq -r .token)

MODELS="[]"
[ -n "$CHAT_MODEL" ] && MODELS=$(jq -c --arg m "$CHAT_MODEL" '[{"id":$m,"capabilities":["chat"]}] + . ' <<<"$MODELS")
[ -n "$EMBED_MODEL" ] && MODELS=$(jq -c --arg m "$EMBED_MODEL" '. + [{"id":$m,"capabilities":["embedding"]}]' <<<"$MODELS")

PID=$(curl -fsS -X POST "http://127.0.0.1:$PORT/settings/llm/providers" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d "{\"name\":\"verify-real\",\"base_url\":\"$BASE_URL\",\"api_key\":\"$API_KEY\",\"models\":$MODELS,\"is_default\":true}" | jq -r .id)
echo "   provider id=$PID"

echo "== 4. 连通测试（chat + embedding 真实调用）"
curl -fsS -X POST "http://127.0.0.1:$PORT/settings/llm/providers/$PID/test" \
  -H "authorization: Bearer $TOKEN" | jq .

echo "== 5. 用量记账验证"
curl -fsS "http://127.0.0.1:$PORT/llm/usage" -H "authorization: Bearer $TOKEN" \
  | jq '[.[] | select(.purpose=="test")] | {rows: length, tokens: [.[] | .input_tokens]}'

echo "== 6. 密钥加密落库验证（明文不得出现在库里）"
COUNT=$(docker exec am-verify-pg psql -U postgres -d am -tAc \
  "SELECT count(*) FROM llm_providers WHERE encode(api_key_encrypted,'escape') LIKE '%$API_KEY%'" 2>/dev/null || echo 1)
[ "$COUNT" = "0" ] && echo "   密文落库 ✓" || { echo "   ✗ 明文泄漏!"; exit 1; }

echo "ALL OK"
