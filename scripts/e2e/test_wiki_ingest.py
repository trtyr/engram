"""E2E #6：Wiki 摄取——purpose → 两步 ingest → 页面/互链/溯源/source ready/review 系统。

review 项是 LLM 非确定性输出，本体断言用 psql 预置一条确定性 review 并走 resolve 全流程；
真 LLM 产出的 review（若有）额外校验合法性。
"""

import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import Client

TEXT = """# tokio 运行时

tokio 是 Rust 生态的主流异步运行时，采用 work-stealing 调度器。
任务通过 spawn 提交，由多线程 worker 窃取执行。
"""


def _gw_ok(e) -> bool:
    if not e.llm_api_key:
        return False
    try:
        c = rq.post(f"{e.llm_base_url}/v1/chat/completions",
                    headers={"Authorization": f"Bearer {e.llm_api_key}"},
                    json={"model": e.llm_chat_model,
                          "messages": [{"role": "user", "content": "ping"}],
                          "max_tokens": 4}, timeout=60)
        g = rq.post(f"{e.llm_base_url}/v1/embeddings",
                    headers={"Authorization": f"Bearer {e.llm_api_key}"},
                    json={"model": e.llm_embed_model, "input": ["ping"]}, timeout=30)
        return c.status_code == 200 and g.status_code == 200
    except rq.RequestException:
        return False


def _wait_wiki_jobs(c: Client, after: str, timeout: float = 300.0) -> dict:
    """等 wiki 两步 job 到终态；返回 {kind: status}（不抛错，调用方决定重试）。"""
    cutoff = datetime.fromisoformat(after.replace("Z", "+00:00"))
    deadline = time.time() + timeout
    out: dict = {}
    while time.time() < deadline:
        jobs = c.get("/jobs", params={"kind": "wiki_analyze,wiki_generate", "limit": 20})
        recent = [j for j in jobs
                  if datetime.fromisoformat(j["created_at"].replace("Z", "+00:00")) > cutoff]
        for k in ("wiki_analyze", "wiki_generate"):
            if k in out:
                continue
            term = [j for j in recent
                    if j["kind"] == k and j["status"] in ("succeeded", "failed", "dead")]
            if term:
                out[k] = term[0]["status"]
        if len(out) == 2:
            return out
        time.sleep(3)
    raise TimeoutError(f"wiki 两步 ingest {timeout}s 未到终态（{out}）")


async def main() -> None:
    e = env.ensure()
    if not _gw_ok(e):
        print("SKIP：LLM 网关不可用（wiki 两步 ingest 需要真 chat+embed）")
        return

    admin = Client.login(e.base_url, e.admin_password)
    admin.post("/settings/llm/providers", json={
        "name": "e2e-wiki", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
        "models": [
            {"id": e.llm_chat_model, "capabilities": ["chat"]},
            {"id": e.llm_embed_model, "capabilities": ["embedding"]},
        ], "is_default": True,
    })
    wiki = admin.with_key(admin.create_api_key("e2e-wiki", ["wiki"]))

    section("purpose：写入与读回")
    wiki.put("/wiki/purpose", json={
        "goals": ["验证 E2E 全链路"], "key_questions": ["wiki 两步 ingest 是否完整"],
        "scope": ["仅技术文档"], "thesis": "LLM 可维护高质量互链页面",
    })
    p = wiki.get("/wiki/purpose")
    eq(p["goals"], ["验证 E2E 全链路"], "purpose 读回一致")

    section("两步 ingest（真 LLM，JSON 抖动时换文本重试）")
    ok_run = False
    for attempt in range(3):
        text = TEXT + ("" if attempt == 0 else f"\n（版本备注 {attempt}）")
        t0 = datetime.now(timezone.utc).isoformat()
        acc = wiki.post("/wiki/ingest", json={"title": "tokio 运行时", "text": text}, timeout=60)
        eq(acc.get("skipped"), False, f"第{attempt + 1}次 ingest 未跳过")
        st = _wait_wiki_jobs(wiki, after=t0)
        if st.get("wiki_analyze") == "succeeded" and st.get("wiki_generate") == "succeeded":
            # LLM 偶发不生成 source 摘要页（非失败，但后续断言依赖）——软重试一次
            got = wiki.get("/wiki/pages", params={"limit": 100})
            if attempt < 2 and not any(pg["page_type"] == "source" for pg in got):
                print(f"[retry] 第{attempt + 1}次未生成 source 摘要页，换文本重试")
                continue
            ok_run = True
            break
        print(f"[retry] 第{attempt + 1}次 generate 未成功（{st}），换文本重试")
    ok(ok_run, "两步 ingest 成功（analyze+generate 双 succeeded）")

    section("页面与互链")
    pages = wiki.get("/wiki/pages", params={"limit": 100})
    ok(len(pages) >= 1, f"页面 ≥1（实际 {len(pages)}）")
    types = {pg["page_type"] for pg in pages}
    ok("source" in types, f"source 摘要页存在（类型集 {sorted(types)}）")
    graph = wiki.get("/wiki/graph")
    ok(len(graph["nodes"]) >= 1, f"图节点 ≥1（{len(graph['nodes'])}）")
    if len(pages) >= 2:
        ok(len(graph["edges"]) >= 1, f"页面 ≥2 时存在互链边（{len(graph['edges'])}）")

    section("溯源：frontmatter.sources 指向原料")
    srcs = wiki.get("/wiki/sources")
    ready = [s for s in srcs if s["status"] == "ready"]
    ok(len(ready) >= 1, "原料 ready")
    sid = str(ready[0]["id"])
    traced = [pg for pg in pages
              if isinstance(pg["frontmatter"].get("sources"), list)
              and sid in [str(x) for x in pg["frontmatter"]["sources"]]]
    ok(len(traced) >= 1, f"≥1 页面 frontmatter.sources 溯源到原料（{len(traced)}）")

    section("index 系统页重建")
    index = wiki.get("/wiki/pages/index")
    ok("[" in index["content"], "index 页含页面链接列表")

    section("W2：内容词检索（tsv 含 title+content，非仅 slug）")
    # 「窃取」「提交」出自源文本正文，LLM 摘要页内容必含；不会出现在任何 slug 里。
    # 旧实现 tsv 只嵌 slug——这两词检索零命中。
    resp = wiki.post("/wiki/search", json={"query": "窃取 提交", "max_items": 20})
    hits = resp.get("pages", resp) if isinstance(resp, dict) else resp
    ok(len(hits) >= 1, f"内容词（非 slug 词）检索命中（{len(hits)}）")
    if hits:
        h0 = hits[0]
        ok(bool(h0["slug"]) and bool(h0["content"]), "命中带 slug 与 content")
    # slug 词检索依旧可用（回归保障）
    resp2 = wiki.post("/wiki/search", json={"query": "tokio", "max_items": 20})
    hits2 = resp2.get("pages", resp2) if isinstance(resp2, dict) else resp2
    ok(len(hits2) >= 1, f"slug 词检索仍命中（{len(hits2)}）")

    section("review 系统：确定性预置 + resolve 全流程")
    subprocess.run(
        ["psql", "-h", env.PG_HOST, "-U", env.PG_USER, "-d", env.E2E_DB,
         "-v", "ON_ERROR_STOP=1", "-c",
         f"INSERT INTO wiki_review_items (id, kind, payload, search_queries, source_id, status) "
         f"VALUES (gen_random_uuid(), 'deep_research', "
         f"'{{\"title\":\"核实 tokio 调度器\",\"reason\":\"E2E 预置\"}}'::jsonb, '[]'::jsonb, "
         f"'{sid}', 'open')"],
        check=True, capture_output=True,
    )
    reviews = wiki.get("/wiki/reviews")
    seeded = [r for r in reviews if r["payload"].get("reason") == "E2E 预置"]
    ok(len(seeded) == 1, "预置 review 出现在 open 队列")
    kinds_ok = all(r["kind"] in ("create_page", "deep_research", "skip", "flag")
                   for r in reviews)
    ok(kinds_ok, "全部 review kind 在预定义集合内（防幻觉动作）")
    wiki.post(f"/wiki/reviews/{seeded[0]['id']}/resolve",
              json={"action": "deep_research", "dismiss": False})
    after_r = wiki.get("/wiki/reviews")
    ok(all(r["id"] != seeded[0]["id"] for r in after_r), "resolve 后移出 open 队列")

    section("再次 ingest 同内容 → sha 幂等跳过")
    acc2 = wiki.post("/wiki/ingest",
                     json={"title": "tokio 运行时", "text": TEXT + ("\n（版本备注 0）" if not ok_run else "")},
                     timeout=60)
    eq(acc2.get("skipped"), True, "同 sha 原料秒跳过")


check.run(main)
