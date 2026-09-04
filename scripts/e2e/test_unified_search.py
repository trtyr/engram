"""E2E #8：跨域统一检索——三域种子 → POST /search 三域标签齐全 + 分数归一化。

纯确定性（FTS 通道；不依赖 LLM——无 provider 时三域各自降级仍可检索）。
"""

import io
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _lib import check, env
from _lib.check import ok, section
from _lib.client import Client

DOC = """# pgvector 存储方案

pgvector 扩展为 PostgreSQL 提供 1024 维向量列与 HNSW 索引。
""".encode("utf-8")


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    full = admin.create_api_key("e2e-full", ["memory", "wiki"])
    c = admin.with_key(full)

    section("两域种子（memory 原子 / wiki 文档 / wiki 页）")
    c.post("/memory/atoms", json={
        "kind": "fact", "content": "项目使用 pgvector 存储嵌入向量", "confidence": 0.95,
    })
    r = c.s.post(f"{c.base}/wiki/upload",
                 files={"file": ("pgvector.md", io.BytesIO(DOC), "text/markdown")},
                 timeout=30)
    check.ok(r.status_code in (200, 201), f"知识上传受理（{r.status_code}）")
    import time
    doc_id = r.json()["id"]
    deadline = time.time() + 60
    while time.time() < deadline:
        if c.get(f"/wiki/documents/{doc_id}")["status"] == "ready":
            break
        time.sleep(1)
    c.put("/wiki/pages/pgvector", json={
        "title": "pgvector", "content": "# pgvector\n向量检索扩展，支持混合检索。",
    })

    section("统一检索：三域一次命中")
    res = c.post("/search", json={"query": "pgvector", "limit": 20})
    ok(isinstance(res.get("hits"), list) and len(res["hits"]) >= 3,
       f"命中 ≥3（实际 {len(res.get('hits', []))}）")
    domains = {h["domain"] for h in res["hits"]}
    ok({"memory", "wiki"} <= domains,
       f"三域标签齐全（实际 {sorted(domains)}）")
    ok(all(h["score"] > 0 for h in res["hits"]), "全部 score > 0（RRF 归一化）")
    ok(all("snippet" in h and h["snippet"] for h in res["hits"]), "全部命中带 snippet")

    section("scope 规则：单域 key 也可用")
    mem_only = admin.with_key(admin.create_api_key("e2e-mem", ["memory"]))
    res2 = mem_only.post("/search", json={"query": "pgvector"})
    ok(isinstance(res2.get("hits"), list), "memory-only key 可用 /search")

    section("空 query → 400")
    from _lib.client import ApiError
    try:
        c.post("/search", json={"query": "  "})
        raise check.Fail("应 400")
    except ApiError as err:
        check.ok(err.status == 400, "空 query 拒绝")


check.run(main)
