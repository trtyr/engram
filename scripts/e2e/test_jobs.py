"""E2E #9：任务系统——永久失败 → failed → 事件时间线 → revive 复活 → 权限隔离。

用 SSRF 拒绝的 URL 摄取制造确定性永久失败（127.0.0.1 私网目标）。
"""

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    know = admin.with_key(admin.create_api_key("e2e-jobs", ["knowledge"]))

    section("制造永久失败任务：SSRF 拒绝的 URL 摄取")
    doc = know.post("/knowledge/documents", json={"url": "http://127.0.0.1:9/never"})
    doc_id = doc["id"]

    def _failed_job():
        jobs = know.get("/jobs", params={"kind": "parse_document", "limit": 20})
        for j in jobs:
            if j["payload"].get("document_id") == doc_id and j["status"] in ("failed", "dead"):
                return j
        return None

    job = None
    deadline = time.time() + 60
    while time.time() < deadline and not job:
        job = _failed_job()
        time.sleep(1)
    ok(job is not None, "parse_document 到终态 failed/dead")
    eq(job["status"], "failed", "永久错误直接 failed（不重试）")
    ok("私网" in (job.get("error") or "") or "Private" in (job.get("error") or ""),
       f"错误信息指向 SSRF 拒绝（{job.get('error', '')[:60]}）")

    section("事件时间线：含入队与失败事件")
    events = know.get(f"/jobs/{job['id']}/events")
    ok(len(events) >= 2, f"事件 ≥2（实际 {len(events)}）")
    levels = {ev["level"] for ev in events}
    ok("error" in levels, "存在 error 级事件")

    section("列表过滤：kind+status 组合")
    filtered = know.get("/jobs", params={"kind": "parse_document", "status": "failed", "limit": 10})
    ok(any(j["id"] == job["id"] for j in filtered), "kind+status 过滤命中目标 job")

    section("revive：管理员可复活，API key 被拒")
    admin.post(f"/jobs/{job['id']}/revive")
    revived = know.get(f"/jobs/{job['id']}")
    eq(revived["status"], "pending", "复活后回 pending（attempts 清零）")
    eq(revived["attempts"], 0, "attempts 清零")
    try:
        know.post(f"/jobs/{job['id']}/revive")
        raise check.Fail("API key 应 403")
    except ApiError as err:
        eq(err.status, 403, "revive 仅管理员（403）")

    section("复活后重跑：再次失败（错误不变）")
    final = None
    deadline = time.time() + 60
    while time.time() < deadline:
        final = know.get(f"/jobs/{job['id']}")
        if final["status"] in ("failed", "dead"):
            break
        time.sleep(1)
    eq(final["status"], "failed", "重跑后再次 failed（同一永久错误）")
    doc_after = know.get(f"/knowledge/documents/{doc_id}")
    eq(doc_after["status"], "failed", "文档状态 failed（错误可见）")
    ok(doc_after.get("error"), "文档行带错误信息")


check.run(main)
