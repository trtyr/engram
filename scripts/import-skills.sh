#!/usr/bin/env bash
# 批量导入技能目录到 Engram 技能域（第六域）。
#
# 用法：
#   ./scripts/import-skills.sh <skills 目录> <API base> <Bearer token> [extra tag]
#
# 行为：
#   - 递归收集目录下所有 .md / .markdown 文件（SKILL.md 形态，frontmatter 容错解析）
#   - 子目录名（如 development / review）自动作为额外 tag
#   - 逐文件入库（服务端逐条成败互不阻断，报告每条结果）；已存在 slug 默认报错，
#     加 OVERWRITE=1 覆盖更新（旧内容自动进版本快照，可回滚）
#
# 示例：
#   ./scripts/import-skills.sh ~/.pi/agent/skills http://localhost:8080 amk_xxx
#   OVERWRITE=1 ./scripts/import-skills.sh ~/.pi/agent/skills http://localhost:8080 amk_xxx
set -euo pipefail

DIR="${1:?用法: import-skills.sh <skills 目录> <API base> <token> [extra tag]}"
BASE="${2:?缺少 API base（如 http://localhost:8080）}"
TOKEN="${3:?缺少 Bearer token（amk_ key 需带 skills scope）}"
EXTRA_TAG="${4:-}"
OVERWRITE="${OVERWRITE:-0}"

command -v python3 >/dev/null || { echo "需要 python3"; exit 1; }

# 收集 .md 文件 → [{filename, content}] JSON（python3 负责转义与子目录 tag）
PAYLOAD=$(python3 - "$DIR" "$EXTRA_TAG" "$OVERWRITE" << 'PYEOF'
import json, os, sys

root, extra_tag, overwrite = sys.argv[1], sys.argv[2], sys.argv[3]
docs = []
for dirpath, dirnames, filenames in os.walk(os.path.expanduser(root)):
    dirnames.sort()
    for fn in sorted(filenames):
        if not fn.endswith(('.md', '.markdown')):
            continue
        path = os.path.join(dirpath, fn)
        with open(path, encoding='utf-8', errors='replace') as f:
            content = f.read()
        tags = []
        if extra_tag:
            tags.append(extra_tag)
        rel = os.path.relpath(dirpath, os.path.expanduser(root))
        if rel != '.' and rel not in ('', '/'):
            tags.append(rel.replace(os.sep, '-'))
        docs.append({
            'filename': fn if rel == '.' else f"{rel}-{fn}",
            'content': content,
            **({'tags': tags} if tags else {}),
        })
print(json.dumps({'documents': docs, 'overwrite': overwrite == '1'}, ensure_ascii=False))
PYEOF
)

COUNT=$(echo "$PAYLOAD" | python3 -c "import json,sys; print(len(json.load(sys.stdin)['documents']))")
if [ "$COUNT" -eq 0 ]; then
  echo "目录 $DIR 下没有 .md 文件"
  exit 0
fi
echo "导入 $COUNT 个文件 → $BASE/skills/import（overwrite=$OVERWRITE）"

RESP=$(curl -sS -X POST "$BASE/skills/import" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD")

echo "$RESP" | python3 -c "
import json, sys
r = json.load(sys.stdin)
print(f\"完成：新建 {r['imported']} · 覆盖 {r['updated']} · 失败 {r['failed']}\")
for item in r['items']:
    if item['status'] == 'failed':
        print(f\"  失败 #{item['index']}: {item.get('error')}\")
"
