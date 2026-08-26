"""E2E #4：记忆读取面——分层检索 / context_pack 相关性 / 原子治理 / 画像回滚 / 会话擦除。

不依赖 LLM（FTS 通道即可）；画像回滚与会话擦除的溯源数据用 psql 预置（确定性）。
"""

import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client


def psql(sql: str) -> None:
    r = subprocess.run(
        ["psql", "-h", env.PG_HOST, "-U", env.PG_USER, "-d", env.E2E_DB,
         "-v", "ON_ERROR_STOP=1", "-c", sql],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"psql 失败: {r.stderr.strip()}\nSQL: {sql}")


def psql_scalar(sql: str) -> str:
    r = subprocess.run(
        ["psql", "-h", env.PG_HOST, "-U", env.PG_USER, "-d", env.E2E_DB,
         "-t", "-A", "-c", sql],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"psql 失败: {r.stderr.strip()}")
    return r.stdout.strip()


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    mem = admin.with_key(admin.create_api_key("e2e-read", ["memory"]))

    section("种子原子（相关 vs 无关高热度）")
    rust = mem.post("/memory/atoms", json={"kind": "fact", "content": "用户偏好使用 Rust 语言进行系统编程", "confidence": 0.95})
    cook = mem.post("/memory/atoms", json={"kind": "fact", "content": "用户喜欢在家做中式烹饪", "confidence": 0.9})
    psql(f"UPDATE atoms SET hit_count = 100 WHERE id = '{cook['id']}'")
    ok(rust["status"] == "active", "手工原子直接 active")

    section("分层检索：命中相关层 + 层过滤")
    res = mem.post("/memory/search", json={"query": "Rust"})
    ok(len(res["l1"]) >= 1, f"l1 命中 Rust 原子（{len(res['l1'])}）")
    ok(all("Rust" in (h["snippet"] or "") or "rust" in (h["snippet"] or "").lower() for h in res["l1"]),
       "命中内容含关键词")
    res_l2 = mem.post("/memory/search", json={"query": "Rust", "layers": ["l2"]})
    eq(res_l2["l1"], [], "layers=[l2] 时 l1 为空（层过滤生效）")

    section("context_pack：query 相关优先于 hit_count")
    pack = mem.get("/memory/context", params={"query": "Rust", "budget_items": 10, "budget_chars": 8000})
    ids = [a["id"] for a in pack["atoms"]]
    ok(rust["id"] in ids, "相关原子（hit_count=0）进入 context")
    if cook["id"] in ids:
        ok(ids.index(rust["id"]) < ids.index(cook["id"]), "相关原子排在高热度无关原子之前")
    eq(pack["meta"]["query"], "Rust", "meta.query 回显")

    section("原子治理：内容更新 / 归档 / 恢复 / 非法状态")
    a2 = mem.patch(f"/memory/atoms/{cook['id']}", json={"content": "用户喜欢在家做粤式烹饪"})
    ok("粤式" in a2["content"], "内容更新生效")
    a3 = mem.patch(f"/memory/atoms/{cook['id']}", json={"status": "archived"})
    eq(a3["status"], "archived", "归档生效")
    a4 = mem.patch(f"/memory/atoms/{cook['id']}", json={"status": "active"})
    eq(a4["status"], "active", "恢复 active")
    try:
        mem.patch(f"/memory/atoms/{cook['id']}", json={"status": "superseded"})
        raise check.Fail("superseded 应 400")
    except ApiError as err:
        eq(err.status, 400, "手工置 superseded 被拒（走矛盾流程）")

    section("画像回滚：历史不可变 + 以新版本落地（psql 预置 v1/v2）")
    psql("INSERT INTO persona_aspects (id, aspect, content, version, prompt_version) VALUES "
         "(gen_random_uuid(), 'preferences', '偏好v1：喜欢深色主题', 1, 'e2e'), "
         "(gen_random_uuid(), 'preferences', '偏好v2：喜欢浅色主题', 2, 'e2e')")
    rolled = mem.post("/memory/persona/rollback", json={"aspect": "preferences", "to_version": 1})
    eq(rolled["version"], 3, "回滚落地为新版本 v3")
    ok("深色" in rolled["content"], "v3 内容回滚到 v1")
    hist = mem.get("/memory/persona/history", params={"aspect": "preferences"})
    eq(len(hist), 3, "历史 3 个版本全保留（不可变）")

    section("会话擦除：来源引用标记 erased（psql 预置溯源原子）")
    s = mem.post("/memory/sessions", json={
        "agent": "e2e", "turns": [{"speaker": "user", "text": "临时会话"}], "distill": "off",
    })
    psql(f"INSERT INTO atoms (id, kind, content, status, source_refs, tsv) VALUES "
         f"(gen_random_uuid(), 'fact', '来自临时会话的事实', 'active', "
         f"'[{{\"session_id\":\"{s['id']}\"}}]'::jsonb, to_tsvector('simple', '临时会话 事实'))")
    r = mem.delete(f"/memory/sessions/{s['id']}")
    ok(r is None, "擦除 204")
    atoms = mem.get("/memory/atoms", params={"limit": 50})
    marked = [a for a in atoms if a["content"] == "来自临时会话的事实"]
    ok(len(marked) == 1 and marked[0]["source_refs"][0].get("erased") is True,
       "来源引用被标记 erased=True（保留结构不删数据）")
    try:
        mem.get(f"/memory/sessions/{s['id']}")
        raise check.Fail("应 404")
    except ApiError as err:
        eq(err.status, 404, "擦除后 GET 会话 404")


check.run(main)
