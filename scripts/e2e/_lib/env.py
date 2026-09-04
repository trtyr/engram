"""环境编排：本机 PG 建独立 E2E 库 → 起服务 → 健康等待 → 清理。零 Docker。

用法（每个 test_*.py 内）：
    from _lib import env
    e = env.E2EEnv.ensure()          # 幂等，起好后返回
    ... 用 e.base_url / e.admin_password / e.master_key ...
    # 进程退出时自动清理（杀服务进程；E2E_KEEP=1 时保留现场排障）

LLM 凭证从环境变量读（LLM 相关测试需要，缺失则该测试 skip）：
    E2E_LLM_BASE_URL     默认 https://newapi.trtyr.top
    E2E_LLM_API_KEY      必需（不落盘、不进代码）
    E2E_LLM_CHAT_MODEL   默认 MiniMax-M3
    E2E_LLM_EMBED_MODEL  默认 BAAI/bge-m3
"""

from __future__ import annotations

import atexit
import os
import secrets
import socket
import subprocess
import sys
import time
from pathlib import Path

import requests

# server/ 与 scripts/ 的相对位置：scripts/e2e/_lib/env.py → 上三级是仓库根
REPO_ROOT = Path(__file__).resolve().parents[3]
SERVER_DIR = REPO_ROOT / "server"
BIN = SERVER_DIR / "target" / "debug" / "engram-server"

E2E_DB = "engram_e2e"
PG_HOST = "127.0.0.1"
PG_PORT = 5432
PG_USER = os.environ.get("USER") or "postgres"
DATABASE_URL = f"postgres://{PG_USER}@{PG_HOST}:{PG_PORT}/{E2E_DB}"

KEEP = os.environ.get("E2E_KEEP") == "1"


def log(msg: str) -> None:
    print(f"[env] {msg}", file=sys.stderr, flush=True)


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _psql(sql: str) -> None:
    """对维护库执行 SQL（建/删 E2E 库）。"""
    subprocess.run(
        ["psql", "-h", PG_HOST, "-p", str(PG_PORT), "-U", PG_USER, "-d", "postgres",
         "-v", "ON_ERROR_STOP=1", "-c", sql],
        check=True, capture_output=True,
    )


class E2EEnv:
    """一次 E2E 运行的全部环境句柄。"""

    def __init__(self) -> None:
        self.base_url = ""
        self.admin_password = ""
        self.master_key = ""
        self.srv: subprocess.Popen | None = None
        self.logfile: Path | None = None

    # ---- LLM 凭证（不落盘） ----

    @property
    def llm_base_url(self) -> str:
        return os.environ.get("E2E_LLM_BASE_URL", "https://newapi.trtyr.top")

    @property
    def llm_api_key(self) -> str | None:
        return os.environ.get("E2E_LLM_API_KEY") or None

    @property
    def llm_chat_model(self) -> str:
        return os.environ.get("E2E_LLM_CHAT_MODEL", "MiniMax-M3")

    @property
    def llm_embed_model(self) -> str:
        return os.environ.get("E2E_LLM_EMBED_MODEL", "BAAI/bge-m3")

    # ---- 生命周期 ----

    def tail_log(self, lines: int = 40) -> str:
        if self.logfile and self.logfile.exists():
            return "\n".join(self.logfile.read_text(errors="replace").splitlines()[-lines:])
        return "(无服务日志)"

    def teardown(self) -> None:
        if KEEP:
            log(f"保留现场：服务 pid={self.srv.pid if self.srv else '-'} 日志={self.logfile}")
            return
        if self.srv and self.srv.poll() is None:
            self.srv.terminate()
            try:
                self.srv.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.srv.kill()
        log("已清理（服务进程已停，E2E 库保留供复用，下次运行会重建）")


_ENV: E2EEnv | None = None


def ensure() -> E2EEnv:
    """幂等启动：建库 → 编译 → 起服务 → 等 ready。进程退出自动清理。"""
    global _ENV
    if _ENV:
        return _ENV

    env = E2EEnv()
    log(f"重建 E2E 库 {E2E_DB}（本机 PG {PG_HOST}:{PG_PORT}，与日常库完全隔离）")
    _psql(f"DROP DATABASE IF EXISTS {E2E_DB}")
    _psql(f"CREATE DATABASE {E2E_DB}")

    log("cargo build（增量）")
    subprocess.run(
        ["cargo", "build", "-p", "engram-api"],
        cwd=SERVER_DIR, check=True, capture_output=True,
    )
    if not BIN.exists():
        raise RuntimeError(f"未找到服务二进制：{BIN}")

    port = _free_port()
    env.base_url = f"http://127.0.0.1:{port}"
    env.admin_password = "e2e-admin-" + secrets.token_hex(4)
    env.master_key = secrets.token_hex(32)

    logfile = Path(f"/tmp/am-e2e-server-{port}.log")
    env.logfile = logfile
    log(f"起服务 :{port}（日志 {logfile}）")
    env.srv = subprocess.Popen(
        [str(BIN)],
        cwd=SERVER_DIR,
        env={
            **os.environ,
            "AGENT_MEMORY_DATABASE_URL": DATABASE_URL,
            "AGENT_MEMORY_PORT": str(port),
            "AGENT_MEMORY_ADMIN_PASSWORD": env.admin_password,
            "AGENT_MEMORY_MASTER_KEY": env.master_key,
        },
        stdout=logfile.open("w"),
        stderr=subprocess.STDOUT,
    )
    atexit.register(env.teardown)

    deadline = time.time() + 60
    while time.time() < deadline:
        try:
            if requests.get(f"{env.base_url}/ready", timeout=2).status_code == 200:
                log("ready ✓（迁移已由服务自动执行）")
                _ENV = env
                return env
        except requests.RequestException:
            pass
        if env.srv.poll() is not None:
            raise RuntimeError(f"服务提前退出 code={env.srv.returncode}\n{env.tail_log()}")
        time.sleep(0.5)
    raise RuntimeError(f"服务 60s 未 ready\n{env.tail_log()}")
