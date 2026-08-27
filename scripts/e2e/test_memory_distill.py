"""E2E #3：真 LLM 蒸馏链全流程——写会话 → extract/arbitrate/organize/persona → 矛盾 supersede。

验证 L0→L1→L2→L3 全链：
- 轮1（建立事实：上海/Mac/Rust/简洁偏好）→ L1 原子 + L2 场景 + L3 画像
- 轮2（矛盾：搬到北京）→ 旧"上海"原子被 supersede，新"北京"原子生效
"""

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import ok, section
from _lib.client import Client

CHAIN = ["extract_atoms", "arbitrate_atoms", "organize_scenarios", "distill_persona"]
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


def _wait_chain(c: Client, after: str | None, required: tuple, timeout: float = 420.0) -> None:
    """等蒸馏链：required 的 kind 必须出现 succeeded；其余 kind 有则必须 succeeded、无则跳过
    （arbitrate 全判重时不入队 organize——条件链语义）。"""
    from datetime import datetime

    def _ts(j):
        return datetime.fromisoformat(j["created_at"].replace("Z", "+00:00"))

    cutoff = datetime.fromisoformat(after.replace("Z", "+00:00")) if after else None
    deadline = time.time() + timeout
    seen: set = set()      # 已 succeeded
    absent: set = set()    # 确认无 job（上游已定）
    while time.time() < deadline:
        jobs = c.get("/jobs", params={"kind": ",".join(CHAIN), "limit": 50})
        recent = [j for j in jobs if cutoff is None or _ts(j) > cutoff]
        for kind in CHAIN:
            if kind in seen:
                continue
            mine = [j for j in recent if j["kind"] == kind]
            term = [j for j in mine if j["status"] in ("succeeded", "failed", "dead")]
            if term:
                j = term[0]
                if j["status"] != "succeeded":
                    ev = c.get(f"/jobs/{j['id']}/events")
                    raise check.Fail(
                        f"{kind} 终态 {j['status']}: {j.get('error')}\n"
                        + "\n".join(f"  {x.get('message')}" for x in ev[:8]))
                seen.add(kind)
            elif kind not in absent and not any(j["status"] in ("pending", "running") for j in mine):
                # 无在途 job：上游已终结（或该 kind 本就是链头）→ 判 absent
                idx = CHAIN.index(kind)
                upstream = CHAIN[idx - 1] if idx > 0 else None
                if upstream is None or (upstream in seen or upstream in absent):
                    absent.add(kind)
        miss_req = [k for k in required if k not in seen]
        in_flight = any(
            j["status"] in ("pending", "running")
            for j in recent if j["kind"] not in seen
        )
        if not miss_req and not in_flight:
            return
        time.sleep(3)
    raise TimeoutError(f"蒸馏链 {timeout}s 未完成，缺 {miss_req}（成功 {sorted(seen)}）")


async def main() -> None:
    e = env.ensure()
    if not e.llm_api_key or not _gateway_ok(e):
        print("SKIP：LLM 网关不可用（蒸馏链需要真 chat+embed）")
        return

    admin = Client.login(e.base_url, e.admin_password)
    admin.post("/settings/llm/providers", json={
        "name": "e2e-distill", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
        "models": [
            {"id": e.llm_chat_model, "capabilities": ["chat"]},
            {"id": e.llm_embed_model, "capabilities": ["embedding"]},
        ], "is_default": True,
    })
    mem = admin.with_key(admin.create_api_key("e2e-distill", ["memory"]))

    section("轮1：写会话（上海 / Mac / Rust / 简洁偏好）并触发蒸馏")
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

    jobs = mem.post("/memory/distill", json={"full": False})
    ok(len(jobs) >= 1 and jobs[0]["kind"] == "extract_atoms", "手动触发返回 extract job")
    _wait_chain(mem, after=None, required=CHAIN)
    ok(True, "链 extract→arbitrate→organize→persona 全部 succeeded")

    section("L1：原子存在 + kind 合法")
    atoms = mem.get("/memory/atoms", params={"status": "active", "limit": 50})
    ok(len(atoms) >= 1, f"active 原子 ≥1（实际 {len(atoms)}）")
    kinds = {a["kind"] for a in atoms}
    ok(kinds <= KINDS_VALID, f"kind 全在枚举内（{sorted(kinds)}）")
    joined = " ".join(a["content"] for a in atoms)
    ok(any(k in joined for k in ("rust", "mac", "上海", "陆家嘴", "简洁")),
       "原子内容含轮1关键信息")

    section("L2：场景生成")
    scenarios = mem.get("/memory/scenarios")
    ok(len(scenarios) >= 1, f"场景 ≥1（实际 {len(scenarios)}）")
    ok(all(s["summary"] for s in scenarios), "场景 summary 非空")

    section("L3：画像出现")
    persona = mem.get("/memory/persona")
    ok(len(persona) >= 1, f"画像分面 ≥1（实际 {len(persona)}）")
    ok(all("evidence_refs" in p and "prompt_version" in p for p in persona),
       "画像行含 evidence_refs / prompt_version（可溯源字段）")

    section("轮2：矛盾会话（搬到北京）→ 再蒸馏（LLM 仲裁非确定性，允许一次更锋利的重试）")
    from datetime import datetime, timezone

    def contradiction_round(text: str) -> bool:
        """写一轮矛盾会话并蒸馏；返回矛盾是否落地（北京存在且上海全 superseded）。"""
        mem.post("/memory/sessions", json={
            "agent": "e2e",
            "turns": [
                {"speaker": "user", "text": text},
                {"speaker": "assistant", "text": "好的，居住地已更新为北京。"},
            ],
            "distill": "off",
        })
        t = datetime.now(timezone.utc).isoformat()
        mem.post("/memory/distill", json={"full": False})
        _wait_chain(mem, after=t, required=("extract_atoms", "arbitrate_atoms"))
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
    ok(settled, "矛盾仲裁落地：新「北京」原子生效")

    all_atoms = mem.get("/memory/atoms", params={"limit": 100})
    shanghai = [a for a in all_atoms
                if "上海" in a["content"] and "北京" not in a["content"]]
    beijing = [a for a in all_atoms if "北京" in a["content"]]
    ok(len(beijing) >= 1, f"新原子含「北京」（实际 {len(beijing)}）")
    if shanghai:
        superseded = [a for a in shanghai if a["status"] == "superseded" and a.get("superseded_by")]
        ok(len(superseded) >= 1, "supersede 指向新原子（superseded_by 已设）")

    section("B1：长会话分段覆盖率（多段事件 + 末段事实不丢）")
    # 构造 >6000 字符的多段输入：中段用背景填充撑体积，首尾各放一个独特标记事实
    filler = "这是一段较长的背景铺垫，讲述日常工作流的细节，用于撑大输入体积，本身无需记忆。" * 2
    long_turns = [{"speaker": "user", "text": "请记住：我的座右铭是「静水流深」。"},
                  {"speaker": "assistant", "text": "好的，已记下。"}]
    for i in range(60):
        long_turns.append({"speaker": "user", "text": f"背景细节 {i}：{filler}"})
    long_turns.append({"speaker": "user", "text": "最后再记一条：我的幸运数字是 42。"})
    long_turns.append({"speaker": "assistant", "text": "好的，幸运数字 42 已记下。"})

    t3 = datetime.now(timezone.utc).isoformat()
    mem.post("/memory/sessions", json={"agent": "e2e", "turns": long_turns, "distill": "off"})
    mem.post("/memory/distill", json={"full": False})
    _wait_chain(mem, after=t3, required=("extract_atoms",))

    # 段级事件留痕（B1 机械断言）：长输入应产生 ≥2 个「分段抽取」事件
    extract_jobs = [j for j in mem.get("/jobs", params={"kind": "extract_atoms", "limit": 5})
                    if j["created_at"] > t3]
    events = mem.get(f"/jobs/{extract_jobs[0]['id']}/events")
    seg_events = [e for e in events if e["message"].startswith("分段抽取")]
    ok(len(seg_events) >= 2, f"长输入分段 ≥2（实际 {len(seg_events)} 段：{[e['message'] for e in seg_events]}）")

    # 末段事实不丢（覆盖率的直接证据：旧单调用下最易静默丢的就是末段）
    _wait_chain(mem, after=t3, required=("extract_atoms", "arbitrate_atoms"))
    final_atoms = mem.get("/memory/atoms", params={"limit": 200})
    tail_fact = [a for a in final_atoms if "42" in a["content"]]
    head_fact = [a for a in final_atoms if "静水流深" in a["content"]]
    ok(len(tail_fact) >= 1, f"末段事实「幸运数字 42」被抽取（{len(tail_fact)}）")
    ok(len(head_fact) >= 1, f"首段事实「座右铭」被抽取（{len(head_fact)}）")


check.run(main)
