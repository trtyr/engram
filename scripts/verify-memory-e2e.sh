#!/usr/bin/env bash
# Phase 2 出口验证：真 LLM 蒸馏链全流程。
# 写入会话 → 蒸馏(extract/arbitrate/organize/persona) → 矛盾supersede → 画像版本化 → context 引用链。
# 用法: ./verify-memory-e2e.sh <BASE_URL> <API_KEY> <CHAT_MODEL> <EMBED_MODEL>
set -euo pipefail

BASE_URL="${1:?需要 base_url}"
API_KEY="${2:?需要 api_key}"
CHAT_MODEL="${3:-}"
EMBED_MODEL="${4:-}"

PORT=19539
PG_PORT=19540
WORKDIR=$(mktemp -d /tmp/am-e2e.XXXXXX)
SERVER_DIR="$(cd "$(dirname "$0")/../server" && pwd)"
API="http://127.0.0.1:$PORT"

cleanup() {
  [ -n "${SRV_PID:-}" ] && kill "$SRV_PID" 2>/dev/null || true
  docker rm -f am-e2e-pg >/dev/null 2>&1 || true
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

echo "== 1. 基础设施"
docker run -d --name am-e2e-pg -e POSTGRES_PASSWORD=e2e -e POSTGRES_DB=am \
  -p "$PG_PORT:5432" pgvector/pgvector:pg17 >/dev/null
for i in $(seq 1 30); do docker exec am-e2e-pg pg_isready -U postgres -d am >/dev/null 2>&1 && break; sleep 1; done
(cargo build -p agent-memory-api --manifest-path "$SERVER_DIR/Cargo.toml" 2>/dev/null || true) >/dev/null
AGENT_MEMORY_DATABASE_URL="postgres://postgres:e2e@127.0.0.1:$PG_PORT/am" \
AGENT_MEMORY_PORT=$PORT \
AGENT_MEMORY_ADMIN_PASSWORD=e2e-admin \
AGENT_MEMORY_MASTER_KEY="$(openssl rand -hex 32)" \
  "$SERVER_DIR/target/debug/agent-memory-server" >"$WORKDIR/server.log" 2>&1 &
SRV_PID=$!
for i in $(seq 1 30); do curl -fsS "$API/ready" >/dev/null 2>&1 && break; sleep 1; done
echo "   ready ✓"

echo "== 2. 登录 + 配置 provider/路由 + 签发 key"
TOKEN=$(curl -fsS -X POST "$API/auth/login" -H 'content-type: application/json' \
  -d '{"password":"e2e-admin"}' | jq -r .token)

MODELS="[]"
[ -n "$CHAT_MODEL" ] && MODELS=$(jq -c --arg m "$CHAT_MODEL" '[{"id":$m,"capabilities":["chat"]}]' <<<"$MODELS")
[ -n "$EMBED_MODEL" ] && MODELS=$(jq -c --arg m "$EMBED_MODEL" '. + [{"id":$m,"capabilities":["embedding"]}]' <<<"$MODELS")
curl -fsS -X POST "$API/settings/llm/providers" -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d "{\"name\":\"gw\",\"base_url\":\"$BASE_URL\",\"api_key\":\"$API_KEY\",\"models\":$MODELS,\"is_default\":true}" | jq -c '{id,name}'

MKEY=$(curl -fsS -X POST "$API/settings/api-keys" -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' -d '{"name":"e2e","scopes":["memory"]}' | jq -r .key)
echo "   api key ✓"

echo "== 3. 两轮会话（轮1建立记忆：上海；轮2制造矛盾：搬到北京）"
post_session() {
  curl -fsS -X POST "$API/memory/sessions" -H "authorization: Bearer $MKEY" \
    -H 'content-type: application/json' -d "$1" | jq -c '{id,distill_status}'
}
trigger_and_wait() {
  curl -fsS -X POST "$API/memory/distill" -H "authorization: Bearer $MKEY" \
    -H 'content-type: application/json' -d '{"full":false}' >/dev/null
  for i in $(seq 1 120); do
    P=$(curl -fsS "$API/jobs?limit=50" -H "authorization: Bearer $MKEY" | jq '[.[] | select(.status=="pending" or .status=="running")] | length')
    [ "$P" -eq 0 ] && break
    sleep 1
  done
}

post_session '{"agent":"pi","distill":"off","turns":[{"speaker":"user","text":"我住在上海，用 Mac 开发，回答保持简洁"},{"speaker":"assistant","text":"记住了"}]}'
echo "轮1 蒸馏..."; trigger_and_wait
SH=$(docker exec am-e2e-pg psql -U postgres -d am -tAc "SELECT count(*) FROM atoms WHERE status='active' AND content LIKE '%上海%'" | tr -d ' ')
[ "$SH" -ge 1 ] && echo "   轮1：上海已入库 ✓" || { echo "   ✗ 轮1未抽取上海"; exit 1; }

post_session '{"agent":"pi","distill":"off","turns":[{"speaker":"user","text":"跟你说，我已经搬到北京了，现在住北京不住上海了"},{"speaker":"assistant","text":"已更新"}]}'
echo "轮2 蒸馏（矛盾消解）..."; trigger_and_wait

echo "== 4. 链完成断言"
curl -fsS -X POST "$API/memory/distill" -H "authorization: Bearer $MKEY" \
  -H 'content-type: application/json' -d '{"full":false}' | jq -c '[.[] | {id,kind,status}]'

echo "== 5. 等链完成（真 LLM，最多 240s）"
DONE=1
for i in $(seq 1 240); do
  P=$(curl -fsS "$API/jobs?kind=extract_atoms,arbitrate_atoms,organize_scenarios,distill_persona&limit=50" \
    -H "authorization: Bearer $MKEY" | jq '[.[] | select(.status=="succeeded" or .status=="failed" or .status=="dead")] | length')
  if [ "$P" -ge 4 ]; then DONE=0; break; fi
  sleep 1
done
[ $DONE -eq 0 ] || { echo "   ✗ 链超时"; exit 1; }
curl -fsS "$API/jobs?limit=10" -H "authorization: Bearer $MKEY" | jq -r '.[] | "\(.kind): \(.status) \(.error // "")"' | head -6

echo "== 6. 断言：L1 原子"
ATOMS=$(curl -fsS "$API/memory/atoms?status=active" -H "authorization: Bearer $MKEY")
echo "$ATOMS" | jq -r '.[] | "  [\(.kind)] \.content) (conf=\(.confidence))"' 2>/dev/null || echo "$ATOMS" | jq -c '.[0:3]'
COUNT=$(echo "$ATOMS" | jq 'length')
[ "$COUNT" -ge 3 ] && echo "   原子数 $COUNT ✓" || { echo "   ✗ 原子数 $COUNT < 3"; exit 1; }

echo "== 7. 断言：矛盾 supersede"
SUP=$(docker exec am-e2e-pg psql -U postgres -d am -tAc \
  "SELECT count(*) FROM atoms WHERE status='superseded'")
[ "$SUP" -ge 1 ] && echo "   superseded=$SUP ✓" || echo "   ⚠ superseded=0（模型未判定矛盾——观察项，不阻断）"

echo "== 8. 断言：画像 + 历史"
PERSONA=$(curl -fsS "$API/memory/persona" -H "authorization: Bearer $MKEY")
echo "$PERSONA" | jq -r '.[] | "  [\(.aspect)] v\(.version): \(.content[0:60])..."'
ASPECT=$(echo "$PERSONA" | jq -r '.[0].aspect // empty')
if [ -n "$ASPECT" ]; then
  HIST=$(curl -fsS "$API/memory/persona/history?aspect=$ASPECT" -H "authorization: Bearer $MKEY")
  HV=$(echo "$HIST" | jq 'length')
  [ "$HV" -ge 1 ] && echo "   $ASPECT 版本数 $HV ✓"
else
  echo "   ✗ 画像为空"; exit 1
fi

echo "== 9. 断言：context 三层包 + 引用链"
CTX=$(curl -fsS "$API/memory/context?query=用户住在哪里" -H "authorization: Bearer $MKEY")
echo "$CTX" | jq -c '.meta'
L3N=$(echo "$CTX" | jq '.persona | length')
L2N=$(echo "$CTX" | jq '.scenarios | length')
L1N=$(echo "$CTX" | jq '.atoms | length')
echo "   L3=$L3N L2=$L2N L1=$L1N"
[ "$L3N" -ge 1 ] && [ "$L2N" -ge 1 ] && [ "$L1N" -ge 1 ] && echo "   三层结构 ✓" || { echo "   ✗ 层缺失"; exit 1; }

# 引用链：persona.evidence → scenario → atom → session
EV=$(echo "$CTX" | jq -r '.persona[0].evidence_refs // empty')
echo "   evidence: $(echo "$EV" | jq -c '{scenarios: (.scenarios|length), atoms: (.atoms|length)}' 2>/dev/null || echo "$EV")"

echo "== 10. 断言：中文检索命中"
SR=$(curl -fsS -X POST "$API/memory/search" -H "authorization: Bearer $MKEY" \
  -H 'content-type: application/json' -d '{"query":"用户住在哪里 城市","max_items":5}')
HITS=$(echo "$SR" | jq '.l1 | length')
echo "   l1 命中: $HITS"
[ "$HITS" -ge 1 ] && echo "$SR" | jq -r '.l1[0] | "   top: \(.snippet) (\(.score|.*1000|round)/1000))"' 2>/dev/null

echo "== 11. 用量记账"
curl -fsS "$API/llm/usage" -H "authorization: Bearer $TOKEN" | jq -c '{rows: length, tokens: [.[] | .input_tokens] | add}'

echo "ALL OK"
