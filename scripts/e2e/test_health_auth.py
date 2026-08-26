"""E2E #1：健康检查 + 登录 + 错误体契约。"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client


async def main() -> None:
    e = env.ensure()

    section("健康检查")
    ok(requests.get(f"{e.base_url}/health", timeout=5).status_code == 200, "GET /health → 200")
    ok(requests.get(f"{e.base_url}/ready", timeout=5).status_code == 200, "GET /ready → 200")

    section("登录")
    admin = Client.login(e.base_url, e.admin_password)
    token = admin.s.headers["Authorization"].removeprefix("Bearer ")
    ok(token.startswith("ams_"), "admin 会话 token 为 ams_ 前缀")

    section("错密码 → 401 + 统一错误体")
    r = requests.post(f"{e.base_url}/auth/login", json={"password": "wrong"}, timeout=10)
    eq(r.status_code, 401, "错密码 HTTP 401")
    body = r.json()
    eq(body["error"]["code"], "unauthorized", "错误体 code=unauthorized")
    eq(body["error"]["retryable"], False, "错误体 retryable=false")

    section("无凭证 → 401")
    r = requests.get(f"{e.base_url}/memory/atoms", timeout=10)
    eq(r.status_code, 401, "无 Bearer 访问受保护端点 → 401")

    section("伪造 token → 401")
    r = requests.get(f"{e.base_url}/memory/atoms",
                     headers={"Authorization": "Bearer amk_fake123"}, timeout=10)
    eq(r.status_code, 401, "无效 amk_ 前缀 → 401")

    section("admin 全权限可用")
    ok(isinstance(admin.get("/settings/llm/providers"), list), "admin 列 providers → 200 list")
    ok(isinstance(admin.get("/jobs"), list), "admin 列 jobs → 200 list")

    section("404 契约")
    try:
        admin.get("/jobs/00000000-0000-0000-0000-000000000000")
        raise check.Fail("应 404")
    except ApiError as err:
        eq(err.status, 404, "不存在的 job → 404")
        eq(err.body["error"]["code"], "not_found", "错误体 code=not_found")


check.run(main)
