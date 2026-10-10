"""E2E #3：离线整理链全流程（P019-M3 更新）——写会话 → 手动触发 maintain_memory Agent → 原子落地。

旧版（P015 前）押四阶段链 extract/arbitrate/organize/persona，已随管线退役删除。
现实验证：L0 会话（distill=off）→ 手动 POST /memory/maintain（cron/admin）→
maintain_memory Agent 落 L1 原子 → 矛盾轮（搬到北京）后旧事实不再 active。
"""

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import ok, section
from _lib.client import Client

CHAIN = ["maintain_memory"]
KINDS_VALID = {"preference", "fact", "decision", "event", "insight", "correction", "failure", "convention"}


def _gateway_ok(e) -> bool:
    try:
        c = rq.post(f"{e.llm_base_url}/v1/chat/completions",
                    headers={"Authorization": f"Bearer {e.llm_api_key}"},
                    json={"model": e.llm_chat_model, "messages": [{"role": "user", "content": "ping"}],
                          "max_tokens": 4}, timeout=60)
        g = rq.post(f"{e.llm_base_url}/v1/embeddings",
                    headers={"Authorization": f"Bearer {e.llm_api_key}"},
                    json={"model": e.llm_embed_model, "input": ["ping"]}, timeout=30)
        return c.status_code == 200 and g.status_code == 200
    except rq.RequestException:
        return False


def _wait_maintain(c: Client, after: str | None, timeout: float = 420.0) -> None:
    """等 maintain_memory 任务到达 succeeded（失败/死亡直接抛出带事件详情）。"""
    from datetime import datetime

    def _ts(j):
        return datetime.fromisoformat(j["created_at"].replace("Z", "+00:00"))

    cutoff = datetime.fromisoformat(after.replace("Z", "+00:00")) if after else None
    deadline = time.time() + timeout
    while time.time() < deadline:
        jobs = c.get("/jobs", params={"kind": ",".join(CHAIN), "limit": 50})
        recent = [j for j in jobs if cutoff is None or _ts(j) > cutoff]
        term = [j for j in recent if j["status"] in ("succeeded", "failed", "dead")]
        if term:
            j = term[0]
            if j["status"] != "succeeded":
                ev = c.get(f"/jobs/{j['id']}/events")
                raise check.Fail(
                    f"maintain_memory 终态 {j['status']}: {j.get('error')}\n"
                    + "\n".join(f"  {x.get('message')}" for x in ev[:8]))
            return
        time.sleep(3)
    raise TimeoutError(f"maintain_memory {timeout}s 未完成")


async def main() -> None:
    e = env.ensure()
    if not e.llm_api_key or not _gateway_ok(e):
        print("SKIP：LLM 网关不可用（整理链需要真 chat+embed）")
        return

    admin = Client.login(e.base_url, e.admin_password)
    admin.post("/settings/llm/providers", json={
        "name": "e2e-distill", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
        "model_id": e.llm_chat_model, "capability": "chat", "is_default": True,
    })
    admin.post("/settings/llm/providers", json={
        "name": "e2e-distill-embed", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
        "model_id": e.llm_embed_model, "capability": "embedding", "is_default": True,
    })
    mem = admin.with_key(admin.create_api_key("e2e-distill", ["memory"]))

    section("轮1：写会话（上海 / Mac / Rust / 简洁偏好）并触发整理")
    s1 = mem.post("/memory/sessions", json={
        "agent": "e2e",
        "turns": [
            {"speaker": "user", "text": "记一下：我住在上海，在陆家嘴上班。"},
            {"speaker": "assistant", "text": "好的，已记下您住在上海，在陆家嘴上班。"},
            {"speaker": "user", "text": "我用 Mac 写 Rust，喜欢简洁的中文回答。"},
            {"speaker": "assistant", "text": "了解，偏好已记录。"},
        ],
        "distill": "off",
    })
    ok(str(s1["id"]).count("-") == 4, "会话创建返回 id")
    eq_session = s1["distill_status"]
    check.ok(eq_session == "pending", f"distill=off 会话保持 pending（实际 {eq_session}）")

    from datetime import datetime, timezone

    t1 = datetime.now(timezone.utc).isoformat()
    trig = admin.post("/memory/maintain", json={})
    ok(trig.get("already_running") is False, "手动触发入队 maintain_memory")
    _wait_maintain(admin, after=t1)
    ok(True, "maintain_memory succeeded")

    section("L1：原子存在 + kind 合法")
    atoms = mem.get("/memory/atoms", params={"status": "active", "limit": 50})
    ok(len(atoms) >= 1, f"active 原子 ≥1（实际 {len(atoms)}）")
    kinds = {a["kind"] for a in atoms}
    ok(kinds <= KINDS_VALID, f"kind 全在枚举内（{sorted(kinds)}）")
    joined = " ".join(a["content"] for a in atoms)
    ok(any(k in joined for k in ("rust", "mac", "上海", "陆家嘴", "简洁")),
       "原子内容含轮1关键信息")

    section("画像活文档存在（persona-doc）")
    doc = admin.get("/memory/persona-doc")
    ok("doc" in doc, "persona-doc 返回 doc 字段")

    section("轮2：矛盾会话（搬到北京）→ 再整理（旧事实不再 active）")
    from datetime import datetime, timezone

    def contradiction_round(text: str) -> bool:
        """写一轮矛盾会话并整理；返回矛盾是否落地（北京存在且上海旧事实不再 active）。"""
        mem.post("/memory/sessions", json={
            "agent": "e2e",
            "turns": [
                {"speaker": "user", "text": text},
                {"speaker": "assistant", "text": "好的，居住地已更新为北京。"},
            ],
            "distill": "off",
        })
        t = datetime.now(timezone.utc).isoformat()
        admin.post("/memory/maintain", json={})
        _wait_maintain(admin, after=t)
        all_atoms = mem.get("/memory/atoms", params={"limit": 100})
        beijing = [a for a in all_atoms if "北京" in a["content"]]
        # 旧「住上海」事实 = 含上海且不含北京（搬家新事实会同时含两词，不算旧事实）
        active_sh = [a for a in all_atoms
                     if "上海" in a["content"] and "北京" not in a["content"]
                     and a["status"] == "active"]
        return len(beijing) >= 1 and not active_sh

    texts = [
        "更新一下：我最近搬到北京住了，已经不在上海了。",
        "纠正之前的记录：我早已不住上海，现在定居北京。",
    ]
    settled = contradiction_round(texts[0]) or contradiction_round(texts[1])
    ok(settled, "矛盾落地：新「北京」原子生效，旧上海事实不再 active")

    all_atoms = mem.get("/memory/atoms", params={"limit": 100})
    beijing = [a for a in all_atoms if "北京" in a["content"]]
    ok(len(beijing) >= 1, f"新原子含「北京」（实际 {len(beijing)}）")

    section("B1：长会话分段覆盖率（末段事实不丢）")
    # 构造多段输入：首尾各放一个独特标记事实
    filler = "这是一段较长的背景铺垫，讲述日常工作流的细节，用于撑大输入体积，本身无需记忆。" * 2
    long_turns = [{"speaker": "user", "text": "请记住：我的座右铭是「静水流深」。"},
                  {"speaker": "assistant", "text": "好的，已记下。"}]
    for i in range(60):
        long_turns.append({"speaker": "user", "text": f"背景细节 {i}：{filler}"})
    long_turns.append({"speaker": "user", "text": "最后再记一条：我的幸运数字是 42。"})
    long_turns.append({"speaker": "assistant", "text": "好的，幸运数字 42 已记下。"})

    t3 = datetime.now(timezone.utc).isoformat()
    mem.post("/memory/sessions", json={"agent": "e2e", "turns": long_turns, "distill": "off"})
    admin.post("/memory/maintain", json={})
    _wait_maintain(admin, after=t3)

    all_atoms = mem.get("/memory/atoms", params={"limit": 200})
    joined = " ".join(a["content"] for a in all_atoms)
    ok("静水流深" in joined, "长会话首段事实（座右铭）不丢")
    ok("42" in joined or "幸运数字" in joined, "长会话末段事实（幸运数字）不丢")


check.run(main)
