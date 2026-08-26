"""API client：登录、api-key 签发、请求封装、job 轮询。"""

from __future__ import annotations

import time
from typing import Any

import requests


class ApiError(Exception):
    """非 2xx 响应（携带 status/body，测试断言错误细节用）。"""

    def __init__(self, status: int, body: Any):
        self.status = status
        self.body = body
        super().__init__(f"HTTP {status}: {body}")


class Client:
    def __init__(self, base_url: str, token: str | None = None):
        self.base = base_url.rstrip("/")
        self.s = requests.Session()
        if token:
            self.s.headers["Authorization"] = f"Bearer {token}"

    # ---- 认证 ----

    @classmethod
    def login(cls, base_url: str, password: str) -> "Client":
        r = requests.post(f"{base_url.rstrip('/')}/auth/login", json={"password": password}, timeout=10)
        r.raise_for_status()
        return cls(base_url, r.json()["token"])

    def create_api_key(self, name: str, scopes: list[str]) -> str:
        r = self.req("POST", "/settings/api-keys", json={"name": name, "scopes": scopes})
        return r["key"]

    def with_key(self, api_key: str) -> "Client":
        return Client(self.base, api_key)

    # ---- 请求 ----

    def req(self, method: str, path: str, **kw) -> Any:
        r = self.s.request(method, f"{self.base}{path}", timeout=kw.pop("timeout", 30), **kw)
        if r.status_code >= 300:
            try:
                body = r.json()
            except ValueError:
                body = r.text
            raise ApiError(r.status_code, body)
        return r.json() if r.content and r.headers.get("content-type", "").startswith("application/json") else None

    def get(self, path: str, **kw) -> Any:
        return self.req("GET", path, **kw)

    def post(self, path: str, **kw) -> Any:
        return self.req("POST", path, **kw)

    def put(self, path: str, **kw) -> Any:
        return self.req("PUT", path, **kw)

    def patch(self, path: str, **kw) -> Any:
        return self.req("PATCH", path, **kw)

    def delete(self, path: str, **kw) -> Any:
        return self.req("DELETE", path, **kw)

    # ---- 轮询 ----

    def wait_job(self, job_id: str, timeout: float = 120.0, want: tuple[str, ...] = ("succeeded",)) -> Any:
        """轮询任务直至终态；want 之外的状态（failed/dead）抛错。"""
        deadline = time.time() + timeout
        job = {}
        while time.time() < deadline:
            job = self.get(f"/jobs/{job_id}")
            status = job.get("status")
            if status in ("succeeded", "failed", "dead"):
                if status not in want:
                    events = self.get(f"/jobs/{job_id}/events")
                    raise AssertionError(
                        f"job {job_id} 终态 {status}: {job.get('error')}\n"
                        + "\n".join(f"  {e.get('message')}" for e in events[:10])
                    )
                return job
            time.sleep(0.5)
        raise TimeoutError(f"job {job_id} {timeout}s 未到终态，最后状态 {job.get('status')}")

    def wait_until(self, fn, timeout: float = 60.0, desc: str = "条件") -> Any:
        """轮询任意断言函数 fn()（返回真值即通过）。"""
        deadline = time.time() + timeout
        last = None
        while time.time() < deadline:
            try:
                last = fn()
                if last:
                    return last
            except Exception as e:  # noqa: BLE001 — 轮询期异常吞掉重试
                last = e
            time.sleep(1.0)
        raise TimeoutError(f"等待{desc}超时，最后结果：{last!r}")
