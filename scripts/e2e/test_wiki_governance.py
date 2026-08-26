"""E2E #7：Wiki 治理——human 页保护 / lint 规则 / insights dismiss / archive_query / 级联删除。

除「human 页不被 LLM 覆盖」需真 LLM 外全部确定性（psql 预置状态）。
"""

import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import Client


def psql(sql: str) -> None:
    r = subprocess.run(
        ["psql", "-h", env.PG_HOST, "-U", env.PG_USER, "-d", env.E2E_DB,
         "-v", "ON_ERROR_STOP=1", "-c", sql],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"psql 失败: {r.stderr.strip()}\nSQL: {sql}")


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    wiki = admin.with_key(admin.create_api_key("e2e-gov", ["wiki"]))

    section("human 页：创建与版本递增")
    pg = wiki.put("/wiki/pages/rust-async", json={
        "title": "Rust 异步", "content": "# Rust 异步\n由人工维护的页面。",
    })
    eq(pg["origin"], "human", "人工编辑 origin=human")
    eq(pg["version"], 1, "首版 v1")
    pg2 = wiki.put("/wiki/pages/rust-async", json={
        "title": "Rust 异步", "content": "# Rust 异步\n人工二次编辑。",
    })
    eq(pg2["version"], 2, "再编辑版本递增 v2")
    human_content = pg2["content"]

    section("lint：dead link + orphan（确定性触发）")
    wiki.put("/wiki/pages/孤立测试页", json={
        "title": "孤立测试页", "content": "正文含死链 [[ghost-page-e2e]] 无其他链接。",
    })
    lint = wiki.post("/wiki/lint")
    rules = {(i["rule"], i["slug"]) for i in lint["issues"]}
    ok(("dead_link", "孤立测试页") in rules, "dead_link 规则命中")
    ok(("orphan", "孤立测试页") in rules, "orphan 规则命中")
    ok(lint["checked_pages"] >= 2, f"checked_pages 覆盖全部页（{lint['checked_pages']}）")

    section("lint：broken frontmatter（psql 去掉 sources）")
    psql("UPDATE wiki_pages SET frontmatter = '{}'::jsonb WHERE slug = '孤立测试页'")
    lint2 = wiki.post("/wiki/lint")
    ok(any(i["rule"] == "broken_frontmatter" and i["slug"] == "孤立测试页"
           for i in lint2["issues"]), "broken_frontmatter 规则命中")

    section("insights：孤立页洞察 → dismiss → 不再出现 → reset 恢复")
    ins = wiki.post("/wiki/insights")
    key = f"isolated_page:孤立测试页"
    ok(any(i["key"] == key for i in ins["insights"]), "孤立页洞察出现")
    ok(isinstance(ins["communities"], list) and ins["total_pages"] >= 2, "报告含社区与总页数")
    wiki.post("/wiki/insights/dismiss", json={"key": key})
    ins2 = wiki.post("/wiki/insights")
    ok(all(i["key"] != key for i in ins2["insights"]), "dismiss 后不再出现")
    wiki.post("/wiki/insights/reset")
    ins3 = wiki.post("/wiki/insights")
    ok(any(i["key"] == key for i in ins3["insights"]), "reset 后恢复")

    section("archive_query：问答存档为 queries 页")
    acc = wiki.post("/wiki/queries/archive", json={
        "title": "e2e问答", "question": "什么是 work-stealing？", "answer": "工作窃取调度。",
    }, timeout=30)
    ok(acc.get("skipped") is not None, "存档受理（202）")
    q = wiki.get("/wiki/pages/query-e2e问答")
    eq(q["page_type"], "queries", "queries 页型正确")
    ok("work-stealing" in q["content"], "问答内容落页")

    section("级联删除：摘要页整删 + 共享页摘源 + dead link 清理（psql 预置）")
    sid = "11111111-1111-7111-8111-111111111111"
    sid2 = "22222222-2222-7222-8222-222222222222"
    psql(
        f"INSERT INTO wiki_sources (id, sha256, raw_path, title, status) VALUES "
        f"('{sid}', 'e2e-cascade-1', '/tmp/e2e-1.md', '删除源', 'ready'), "
        f"('{sid2}', 'e2e-cascade-2', '/tmp/e2e-2.md', '保留源', 'ready'); "
        f"INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version) VALUES "
        f"(gen_random_uuid(), 'src-摘要页', '摘要', 'source', '# 摘要', "
        f"'{{\"title\":\"摘要\",\"sources\":[\"{sid}\"]}}'::jsonb, 'llm', 1), "
        f"(gen_random_uuid(), '共享概念页', '概念', 'concept', '参见 [[src-摘要页]]', "
        f"'{{\"title\":\"概念\",\"sources\":[\"{sid}\",\"{sid2}\"]}}'::jsonb, 'llm', 1)"
    )
    report = wiki.delete(f"/wiki/sources/{sid}")
    ok("src-摘要页" in report["deleted_pages"], "摘要页整页删除")
    ok("共享概念页" in report["updated_shared"], "共享页保留仅摘源")
    eq(report["cleaned_links"], 1, "dead wikilink 清理 1 处")
    srcs = {s["id"] for s in wiki.get("/wiki/sources")}
    ok(sid not in srcs and sid2 in srcs, "删源行保留另一源")
    shared = wiki.get("/wiki/pages/共享概念页")
    sources = [str(x) for x in shared["frontmatter"]["sources"]]
    ok(sid not in sources and sid2 in sources, "共享页 sources 仅剩保留源")
    ok("[[src-摘要页]]" not in shared["content"], "正文死链已清理")

    section("human 页不被 LLM 覆盖（有 LLM 时真跑 ingest 验证）")
    if e.llm_api_key:
        try:
            c = rq.post(f"{e.llm_base_url}/v1/chat/completions",
                        headers={"Authorization": f"Bearer {e.llm_api_key}"},
                        json={"model": e.llm_chat_model,
                              "messages": [{"role": "user", "content": "ping"}],
                              "max_tokens": 4}, timeout=60)
            gw = c.status_code == 200
        except rq.RequestException:
            gw = False
        if gw:
            admin.post("/settings/llm/providers", json={
                "name": "e2e-gov", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
                "models": [
                    {"id": e.llm_chat_model, "capabilities": ["chat"]},
                    {"id": e.llm_embed_model, "capabilities": ["embedding"]},
                ], "is_default": True,
            })
            wiki.post("/wiki/ingest", json={
                "title": "Rust 异步补充材料",
                "text": "# Rust 异步补充\nrust-async 是人工维护的核心页面，讲 async/await 与 tokio。\n",
            }, timeout=60)
            import time as _t
            _t.sleep(25)  # 给两步 ingest 留时间（不强等终态：断言的是"未被覆盖"）
            after = wiki.get("/wiki/pages/rust-async")
            eq(after["origin"], "human", "ingest 后 origin 仍 human")
            eq(after["version"], 2, "版本未被 LLM 递增")
            eq(after["content"], human_content, "内容未被覆盖")
        else:
            print("SKIP：网关 chat 不可用，human 保护走代码路径保证")
    else:
        print("SKIP：无 LLM 凭证，human 保护走代码路径保证")


check.run(main)
