"""E2E #10b：LLM 设置——routing 读写 / 回退链生效（ghost provider → 默认兜底）/ 用量查询。

回退链用真 embedding 观测：把 embed 路由指向不存在的 provider，知识摄取仍应
落到默认 provider 完成嵌入（resolve 的回退语义）。
"""

import io
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import Client

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
        "models": [
            {"id": e.llm_chat_model, "capabilities": ["chat"]},
            {"id": e.llm_embed_model, "capabilities": ["embedding"]},
        ],
        "is_default": True,
    })

    section("routing：写入与读回")
    table = {"embed": [{"provider": "ghost-不存在", "model": "whatever"}]}
    admin.put("/settings/llm/routing", json=table)
    back = admin.get("/settings/llm/routing")
    eq(back.get("embed"), table["embed"], "路由表读写一致（配置即时生效）")

    section("回退链：embed 路由指向幽灵 → 摄取仍嵌入成功（默认 provider 兜底）")
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
