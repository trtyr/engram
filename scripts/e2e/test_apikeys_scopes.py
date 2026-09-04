"""E2E #2：api-key 签发 + scope 隔离 + 吊销。"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)

    section("签发 memory-only key")
    raw_key = admin.create_api_key("e2e-mem", ["memory"])
    ok(raw_key.startswith("amk_"), "key 为 amk_ 前缀（明文只出现一次）")
    mem = admin.with_key(raw_key)

    section("scope 内可用")
    ok(isinstance(mem.get("/memory/atoms"), list), "memory key → GET /memory/atoms 200")
    ok(isinstance(mem.get("/memory/sessions"), list), "memory key → GET /memory/sessions 200")

    section("scope 外 → 403")
    for path in ("/wiki/pages", "/wiki/documents", "/codegraph/projects"):
        try:
            mem.get(path)
            raise AssertionError(f"{path} 应 403")
        except ApiError as err:
            eq(err.status, 403, f"{path} → 403")
            eq(err.body["error"]["code"], "forbidden", f"{path} 错误体 code=forbidden")

    section("跨域检索 scope 规则（至少其一）")
    ok(isinstance(mem.post("/search", json={"query": "任意"}), dict),
       "memory key → POST /search 200（有任一域 scope 即可）")

    section("非法 scope 签发 → 400")
    try:
        admin.create_api_key("bad", ["not-a-scope"])
        raise AssertionError("非法 scope 应 400")
    except ApiError as err:
        eq(err.status, 400, "签发非法 scope → 400")

    section("吊销后 → 401")
    keys = admin.get("/settings/api-keys")
    kid = next(k["id"] for k in keys if k["name"] == "e2e-mem")
    admin.post(f"/settings/api-keys/{kid}/revoke")
    try:
        mem.get("/memory/atoms")
        raise AssertionError("吊销后应 401")
    except ApiError as err:
        eq(err.status, 401, "吊销后使用 → 401")


check.run(main)
