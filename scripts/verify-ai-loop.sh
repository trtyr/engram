#!/usr/bin/env bash
# Phase 7 出口验证：AI 视角闭环。
# 模拟一个只读过 docs/AI-INTERFACE.md 的 AI 客户端：
# 登录/签发 key 由管理员完成（现实中人工），AI 仅持 key 操作全部工作流。
set -euo pipefail

BASE="${1:?用法: verify-ai-loop.sh <base_url>}"
ADMIN_PW="${2:?需要管理员密码}"
export E2E_ADMIN_PW="$ADMIN_PW"

API="$BASE"
echo "== 0. 管理员准备（签发 key）"
TOKEN=$(curl -fsS -X POST "$API/auth/login" -H 'content-type: application/json' \
  -d "{\"password\":\"$ADMIN_PW\"}" | jq -r .token)
KEY=$(curl -fsS -X POST "$API/settings/api-keys" -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"name":"ai-loop","scopes":["memory","knowledge","wiki","codegraph"]}' | jq -r .key)
echo "   key 签发 ✓（模拟交给 AI 客户端）"
AUTH="authorization: Bearer $KEY"

echo "== 1. 会话开始：拉上下文"
curl -fsS "$API/memory/context" -H "$AUTH" | jq -c '{l3: (.persona|length), l2: (.scenarios|length), l1: (.atoms|length), meta}'

echo "== 2. 写入会话（含可蒸馏事实）"
curl -fsS -X POST "$API/memory/sessions" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"agent":"ai-loop-test","distill":"off","turns":[{"speaker":"user","text":"记住：我的生产环境数据库是 PostgreSQL 17，部署在上海区"},{"speaker":"assistant","text":"已记录"}]}' | jq -c '{id, distill_status}'

echo "== 3. 触发蒸馏并等待"
curl -fsS -X POST "$API/memory/distill" -H "$AUTH" -H 'content-type: application/json' -d '{"full":false}' >/dev/null
for i in $(seq 1 120); do
  P=$(curl -fsS "$API/jobs?limit=50" -H "$AUTH" | jq '[.[] | select(.status=="pending" or .status=="running")] | length')
  [ "$P" -eq 0 ] && break
  sleep 1
done
curl -fsS "$API/jobs?kind=extract_atoms,arbitrate_atoms,organize_scenarios,distill_persona&limit=10" -H "$AUTH" | jq -r '.[] | "  \(.kind): \(.status)"'

echo "== 4. 检索验证（语义命中）"
curl -fsS -X POST "$API/memory/search" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"query":"数据库 部署 哪里","max_items":5}' | jq -c '{l1_hits: [.l1[] | .snippet][0:3]}'

echo "== 5. 二次上下文（画像应含新信息）"
curl -fsS "$API/memory/context?query=生产环境" -H "$AUTH" | jq -c '{l3: (.persona | map({aspect, has_content: (.content|length>0)}))}'

echo "== 6. 知识摄取（URL）+ 检索"
curl -fsS -X POST "$API/knowledge/documents" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"url":"https://mirrors.tuna.tsinghua.edu.cn/"}' >/dev/null
sleep 20
curl -fsS -X POST "$API/knowledge/search" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"query":"镜像站 开源软件","max_items":3}' | jq -c '[.[] | .document_title][0:2]'

echo "== 7. Wiki ingest + 查询"
curl -fsS -X POST "$API/wiki/ingest" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"title":"AI 记忆系统","text":"agent-memory 平台通过分层蒸馏维护用户画像。蒸馏管道由 extract、arbitrate、organize、persona 四阶段组成。画像按分面版本化存储在 PostgreSQL 中。"}' >/dev/null
sleep 30
curl -fsS -X POST "$API/wiki/search" -H "$AUTH" -H 'content-type: application/json' \
  -d '{"query":"蒸馏 管道 阶段","max_items":3}' | jq -c '[.[] | .slug][0:3]'

echo "== 8. 用量与任务可观测"
curl -fsS "$API/llm/usage" -H "authorization: Bearer $TOKEN" | jq -c '{rows: length, tokens: [.[] | .input_tokens] | add}'
echo "AI 视角闭环 ALL OK"
