"""E2E #10b：LLM 设置——routing 写入校验 / 回退链生效（DB 级幽灵路由 → 默认兜底）/ 用量查询。

L4 后 API 拒绝幽灵路由（400）；resolve 的回退语义改由 psql 直接种路由观测
（校验在 API 层，resolve 读库不经过校验——两个层次分别验证）。
"""

import io

import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client

DOC = ("# 回退链验证\n"
       "路由指向幽灵 provider 时，嵌入应回退到默认 provider 完成。\n").encode("utf-8")


def _embed_ok(e) -> bool:
    if not e.llm_api_key:
        return False
    try:
        r = rq.post(f"{e.llm_base_url}/v1/embeddings",
                    headers={"Authorization": f"Bearer {e.llm_api_key}"},
                    json={"model": e.llm_embed_model, "input": ["ping"]}, timeout=30)
        return r.status_code == 200
    except rq.RequestException:
        return False


def _psql(sql):
    subprocess.run(["psql", "-h", env.PG_HOST, "-U", env.PG_USER, "-d", env.E2E_DB,
                    "-v", "ON_ERROR_STOP=1", "-c", sql],
                   check=True, capture_output=True)


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    know = admin.with_key(admin.create_api_key("e2e-rt", ["knowledge"]))

    has_llm = _embed_ok(e)
    if not has_llm:
        print("SKIP：无可用 embedding 网关（回退链观测需要真嵌入）")
        return
    admin.post("/settings/llm/providers", json={
        "name": "e2e-rt-default", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
        "model_id": e.llm_chat_model, "capability": "chat", "is_default": True,
    })
    admin.post("/settings/llm/providers", json={
        "name": "e2e-rt-embed", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
        "model_id": e.llm_embed_model, "capability": "embedding", "is_default": True,
    })

    section("L4：routing 写入校验（幽灵 provider / typo purpose 拒绝）")
    try:
        admin.put("/settings/llm/routing",
                  json={"embed": [{"provider": "ghost-不存在", "model": "whatever"}]})
        ok(False, "幽灵 provider 路由应被拒")
    except ApiError as ex:
        eq(ex.status, 400, "幽灵 provider 路由被拒（400）")
    try:
        admin.put("/settings/llm/routing",
                  json={"extarct": [{"provider": "e2e-rt-default", "model": e.llm_chat_model}]})
        ok(False, "typo purpose 应被拒")
    except ApiError as ex:
        eq(ex.status, 400, "typo purpose 被拒（400）")
        msg = str(ex.body.get("error", {}).get("message", "")) if isinstance(ex.body, dict) else str(ex.body)
        ok("extarct" in msg, f"报错带违规 purpose 明细（{msg[:60]}）")

    section("回退链：DB 级幽灵路由 → 摄取仍嵌入成功（默认 provider 兜底）")
    # L4 校验在 API 层；resolve 读库不经过校验——psql 直接种幽灵路由观测回退语义
    _psql("INSERT INTO settings (key, value) VALUES ('llm_routing', "
          "'{\"embed\": [{\"provider\": \"ghost-不存在\", \"model\": \"whatever\"}]}'::jsonb) "
          "ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
    back = admin.get("/settings/llm/routing")
    eq(back.get("embed", [{}])[0].get("provider"), "ghost-不存在", "幽灵路由已入库（psql 种子）")

    r = know.s.post(f"{know.base}/knowledge/upload",
                    files={"file": ("fallback.md", io.BytesIO(DOC), "text/markdown")},
                    timeout=30)
    doc_id = r.json()["id"]
    doc = {}
    deadline = time.time() + 120
    while time.time() < deadline:
        doc = know.get(f"/knowledge/documents/{doc_id}")
        if doc["status"] in ("ready", "failed"):
            break
        time.sleep(1)
    eq(doc["status"], "ready", "文档 ready（ghost 路由未阻塞摄取）")
    chunks = know.get(f"/knowledge/documents/{doc_id}/chunks")
    ok(all(not c["embed_failed"] for c in chunks),
       "chunks 嵌入成功（回退到默认 provider）")

    section("清空路由 → 读回为空表")
    admin.put("/settings/llm/routing", json={})
    back2 = admin.get("/settings/llm/routing")
    eq(back2, {}, "空路由表读写一致（全部走默认）")

    section("用量查询（provider 连通探测走记账路径）")
    provs = admin.get("/settings/llm/providers")
    pid = next(x["id"] for x in provs if x["name"] == "e2e-rt-default")
    admin.post(f"/settings/llm/providers/{pid}/test", timeout=120)
    usage = admin.get("/llm/usage")
    ok(isinstance(usage, list) and len(usage) >= 1, "用量记录出现")
    if usage:
        ok(any(u.get("purpose") == "test" for u in usage), "存在 test 用途记账行")


check.run(main)
