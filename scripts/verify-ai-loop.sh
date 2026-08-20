#!/usr/bin/env bash
# Phase 7 出口验证：AI 视角闭环。
# 模拟一个只读过 docs/AI-INTERFACE.md 的 AI 客户端：
# 管理员配置 provider + 签发 key（第 0 步），AI 仅持 key 走完全部工作流。
# 环境变量：AGENT_MEMORY_GW_URL / AGENT_MEMORY_GW_KEY（干净库首次必填）
#           AGENT_MEMORY_GW_CHAT（默认 shanghai/deepseek-v4-flash）
#           AGENT_MEMORY_GW_EMBED（默认 Qwen/Qwen3-Embedding-8B）
set -euo pipefail

BASE="${1:?用法: verify-ai-loop.sh <base_url> <admin_password>}"
ADMIN_PW="${2:?需要管理员密码}"

API="$BASE"
GW_URL="${AGENT_MEMORY_GW_URL:-}"
GW_KEY="${AGENT_MEMORY_GW_KEY:-}"
GW_CHAT="${AGENT_MEMORY_GW_CHAT:-shanghai/deepseek-v4-flash}"
GW_EMBED="${AGENT_MEMORY_GW_EMBED:-Qwen/Qwen3-Embedding-8B}"

echo "== 0. 管理员准备（配置 provider + 签发 key）"
TOKEN=$(curl -fsS -X POST "$API/auth/login" -H 'content-type: application/json' -d "{\"password\":\"$ADMIN_PW\"}" | jq -r .token)
PN=$(curl -fsS "$API/settings/llm/providers" -H "authorization: Bearer $TOKEN" | jq 'length')
if [ "$PN" -eq 0 ]; then
  if [ -z "$GW_URL" ] || [ -z "$GW_KEY" ]; then
    echo "   缺少 provider 且未提供 AGENT_MEMORY_GW_URL/GW_KEY" >&2
    exit 1
  fi
  curl -fsS -X POST "$API/settings/llm/providers" -H "authorization: Bearer $TOKEN" \
    -H 'content-type: application/json' \
    -d "{\"name\":\"gw\",\"base_url\":\"$GW_URL\",\"api_key\":\"$GW_KEY\",\"models\":[{\"id\":\"$GW_CHAT\",\"capabilities\":[\"chat\"]},{\"id\":\"$GW_EMBED\",\"capabilities\":[\"embedding\"]}],\"is_default\":true}" >/dev/null
  echo "   provider 已配置（$GW_CHAT + $GW_EMBED）"
else
  echo "   provider 已存在（$PN 个）"
fi
KEY=$(curl -fsS -X POST "$API/settings/api-keys" -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"name":"ai-loop","scopes":["memory","knowledge","wiki","codegraph"]}' | jq -r .key)
echo "   key 已签发（模拟交给 AI 客户端）"
AUTH="authorization: Bearer $KEY"

echo "== 1. 会话开始：拉上下文"
curl -fsS "$API/memory/context" -H "$AUTH" | jq -c '{l3: (.persona|length), l2: (.scenarios|length), l1: (.atoms|length), meta}'

echo "== 2. 写入会话（含可蒸馏事实）"
curl -fsS -X POST "$API/memory/sessions" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"agent":"ai-loop-test","distill":"off","turns":[{"speaker":"user","text":"记住：我的生产环境数据库是 PostgreSQL 17，部署在上海区"},{"speaker":"assistant","text":"已记录"}]}' | jq -c '{id, distill_status}'

echo "== 3. 触发蒸馏并等待（四阶段）"
curl -fsS -X POST "$API/memory/distill" -H "$AUTH" -H 'content-type: application/json' -d '{"full":false}' >/dev/null
for _ in $(seq 1 180); do
  P=$(curl -fsS "$API/jobs?limit=50" -H "$AUTH" | jq '[.[] | select(.status=="pending" or .status=="running")] | length')
  [ "$P" -eq 0 ] && break
  sleep 1
done
curl -fsS "$API/jobs?kind=extract_atoms,arbitrate_atoms,organize_scenarios,distill_persona&limit=10" -H "$AUTH" | jq -r '.[] | "   \(.kind): \(.status)"'
DISTILL_OK=$(curl -fsS "$API/jobs?kind=extract_atoms,arbitrate_atoms,organize_scenarios,distill_persona&limit=10" -H "$AUTH" | jq '[.[] | select(.status=="succeeded")] | length')
if [ "$DISTILL_OK" -lt 4 ]; then
  echo "   蒸馏未全成（$DISTILL_OK/4）："
  curl -fsS "$API/jobs?kind=extract_atoms,arbitrate_atoms,organize_scenarios,distill_persona&limit=10" -H "$AUTH" | jq -r '.[] | select(.status!="succeeded") | "   \(.kind): \(.error)"'
  exit 1
fi
echo "   蒸馏四阶段全部 succeeded"

echo "== 4. 检索验证（语义命中）"
L1=$(curl -fsS -X POST "$API/memory/search" -H "$AUTH" -H 'content-type: application/json' -d '{"query":"生产环境 数据库","max_items":5}')
echo "$L1" | jq -c '{l1_hits: [.l1[] | .snippet][0:3]}'
L1N=$(echo "$L1" | jq '.l1 | length')
[ "$L1N" -ge 1 ] || { echo "   检索未命中"; exit 1; }
echo "   语义检索命中"

echo "== 5. 二次上下文（画像应含新信息）"
curl -fsS "$API/memory/context?query=生产环境" -H "$AUTH" | jq -c '{l3: (.persona | map({aspect, has_content: (.content|length>0)}))}'

echo "== 6. 知识摄取（URL）+ 检索"
curl -fsS -X POST "$API/knowledge/documents" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"url":"https://mirrors.tuna.tsinghua.edu.cn/"}' >/dev/null
for _ in $(seq 1 90); do
  DS=$(curl -fsS "$API/knowledge/documents?limit=5" -H "$AUTH" | jq -r '[.[] | select(.source_uri | contains("tuna"))][0].status // "none"')
  [ "$DS" = "ready" ] && break
  if [ "$DS" = "failed" ]; then echo "   URL 摄取 failed"; exit 1; fi
  sleep 1
done
echo "   摄取状态: $DS"
KS=$(curl -fsS -X POST "$API/knowledge/search" -H "$AUTH" -H 'content-type: application/json' -d '{"query":"镜像站 开源软件","max_items":3}')
echo "$KS" | jq -c '{doc_hits: [.[] | .document_title][0:2]}'
KSN=$(echo "$KS" | jq 'length')
[ "$KSN" -ge 1 ] || { echo "   知识检索未命中"; exit 1; }
echo "   知识检索命中"

echo "== 7. Wiki ingest + 查询"
curl -fsS -X POST "$API/wiki/ingest" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"title":"AI 记忆系统","text":"agent-memory 平台通过分层蒸馏维护用户画像。蒸馏管道由 extract、arbitrate、organize、persona 四阶段组成。画像按分面版本化存储在 PostgreSQL 中。"}' >/dev/null
for _ in $(seq 1 180); do
  W=$(curl -fsS "$API/jobs?kind=wiki_generate&limit=1" -H "$AUTH" | jq -r '.[0].status // "none"')
  [ "$W" = "succeeded" ] && break
  if [ "$W" = "failed" ] || [ "$W" = "dead" ]; then
    echo "   wiki job $W：" >&2
    curl -fsS "$API/jobs?kind=wiki_generate&limit=1" -H "$AUTH" | jq -r '.[0].error' >&2
    exit 1
  fi
  sleep 1
done
echo "   wiki_generate: $W"
WS=$(curl -fsS -X POST "$API/wiki/search" -H "$AUTH" -H 'content-type: application/json' -d '{"query":"蒸馏","max_items":5}')
echo "$WS" | jq -c '{wiki_hits: [.[] | .slug]}'
WSN=$(echo "$WS" | jq 'length')
[ "$WSN" -ge 1 ] || { echo "   wiki 检索未命中"; exit 1; }
echo "   wiki 检索命中"

echo "== 8. 用量与任务可观测"
curl -fsS "$API/llm/usage" -H "authorization: Bearer $TOKEN" | jq -c '{rows: length, tokens: ([.[] | .input_tokens] | add)}'

echo "AI 视角闭环 ALL OK"
