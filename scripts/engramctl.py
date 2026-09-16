#!/usr/bin/env python3
"""engramctl — engram 宿主进程管理脚本（2026-09-15 起，日常运行形态）。

运行形态（2026-09-16 起）：**运行时家自包含于 ~/.engram/**——数据、配置、二进制
（bin/engram-server）、管理脚本（bin/engramctl）全在本机；代码仓库（外置开发盘）
只是开发工作区，仅 build/install 需要。拔掉外置盘：服务照跑、照重启，只是不能重新构建。

用法：
  python3 scripts/engramctl.py start   [--skip-build]   # PG 检查 → build → 安装 → 后台挂起 → /ready
  python3 scripts/engramctl.py stop                     # 停进程（SIGTERM，10s 后 SIGKILL）
  python3 scripts/engramctl.py status                   # 进程 + /ready + migration 版本
  python3 scripts/engramctl.py logs   [N]               # 看最后 N 行日志（默认 50）
  python3 scripts/engramctl.py restart [--skip-build]   # 先 build 成功 → 安装 → 再 stop + start
  python3 scripts/engramctl.py install                  # 把 target 构建产物装进 ~/.engram/bin（不重启）

配置源：~/.engram/.env（KEY=VALUE）——需含：
  AGENT_MEMORY_ADMIN_PASSWORD / AGENT_MEMORY_MASTER_KEY
  ENGRAM_DATABASE_URL（宿主 PG 连接串，如 postgres://trtyr@127.0.0.1:5432/engram）
可选：AGENT_MEMORY_METRICS、RUST_LOG（默认 info）
"""

from __future__ import annotations  # 兼容 3.9 解析 `int | None` 注解（注解惰性化）

import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

# 版本守卫：空白 macOS 只带系统 python3（3.9），低于 3.10 时给可行动指引而不是一屏 traceback
if sys.version_info < (3, 10):
    print(
        f"❌ Python 版本过旧（{sys.version_info.major}.{sys.version_info.minor}）——本脚本需要 3.10+"
    )
    print("   安装新版：brew install python@3.12")
    print(f"   或显式用新版跑：/opt/homebrew/bin/python3 {' '.join(sys.argv)}")
    sys.exit(1)

HOME = Path.home()
RUNTIME_DIR = HOME / ".engram"
ENV_FILE = RUNTIME_DIR / ".env"
PID_FILE = RUNTIME_DIR / "server.pid"
LOG_FILE = RUNTIME_DIR / "server.log"
# 运行二进制与管理脚本装在本机（~/.engram/bin）——engram 的运行时家自包含，
# 不依赖代码仓库所在卷：拔掉外置开发盘，服务照跑、照重启。
# 代码仓库（SERVER_DIR）只是开发工作区，仅 build/install 时需要。
BIN_DIR = RUNTIME_DIR / "bin"
BINARY = BIN_DIR / "engram-server"
SERVER_DIR = Path(__file__).resolve().parent.parent / "server"
PORT = 17654
READY_URL = f"http://127.0.0.1:{PORT}/ready"
# 前端产物：rust-embed 在**编译期**嵌入 web/dist（crates/api/src/web_assets.rs），
# 目录不存在时 `cargo build` 直接编译失败——所以 build 前先探它，给出可行动的提示。
WEB_DIST = SERVER_DIR.parent / "web" / "dist"


def load_env() -> dict:
    env = {}
    if ENV_FILE.exists():
        for line in ENV_FILE.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, _, v = line.partition("=")
                env[k.strip()] = v.strip()
    return env


def pg_ready(dsn: str) -> bool:
    r = subprocess.run(["psql", dsn, "-tc", "SELECT 1;"], capture_output=True, text=True, timeout=10)
    return r.returncode == 0


def server_pid() -> int | None:
    if PID_FILE.exists():
        try:
            pid = int(PID_FILE.read_text().strip())
            os.kill(pid, 0)  # 活性探测
            return pid
        except (ValueError, ProcessLookupError, PermissionError):
            return None
    return None


def ready_probe() -> dict | None:
    try:
        with urllib.request.urlopen(READY_URL, timeout=3) as resp:
            import json

            return json.loads(resp.read().decode())
    except Exception:
        return None


def build_release() -> int:
    """编译 release 二进制（0=成功）。**不碰正在运行的服务**——失败只返回非零。

    历史教训（2026-09-15）：restart 原先是「先 stop 再 build」，build 一失败服务就留在
    停摆状态（当时 web/dist 缺失导致 build 失败）。所以 build 必须先行、且与停服务解耦。
    """
    if not (SERVER_DIR / "Cargo.toml").is_file():
        print(f"❌ 代码仓库不在：{SERVER_DIR} 下没有 Cargo.toml")
        print("   本脚本是安装版（只带运行时，不带源码）——构建请回到代码仓库用仓库版 engramctl。")
        print("   运行管理（start/stop/status/logs/restart）不受影响。")
        return 1
    if not WEB_DIST.is_dir():
        print("❌ 前端产物缺失：web/dist 不存在")
        print("   web_assets.rs 用 RustEmbed 把前端**编译期**嵌进二进制，cargo build 需要它先存在。修复：")
        print(f"     cd {WEB_DIST.parent} && pnpm install && pnpm build")
        return 1
    print("[2/4] cargo build --release（--skip-build 可跳过）...", flush=True)
    r = subprocess.run(
        ["cargo", "build", "--release", "-p", "engram-api"],
        cwd=SERVER_DIR,
        env={**os.environ, "CARGO_TERM_COLOR": "never"},
    )
    if r.returncode != 0:
        print("❌ build 失败——正在运行的服务未受影响")
        return 1
    return 0


def cmd_install() -> int:
    """把构建产物装进本机运行时家（~/.engram/bin）：二进制 + 管理脚本自身。

    装完后 engram 的**运行**不再依赖代码仓库所在卷——拔掉外置开发盘，服务照跑、
    stop/start/status 照用；仓库只在「重新构建」时才需要。旧二进制留 .prev 一份可回滚。
    """
    src = SERVER_DIR / "target" / "release" / "engram-server"
    if not (SERVER_DIR / "Cargo.toml").is_file():
        print(f"❌ 本脚本是安装版（{Path(__file__).resolve()}），不带安装时对应的代码仓库")
        print("   install 要从仓库的 target/ 拷构建产物——请回到代码仓库用仓库版 engramctl 操作。")
        print("   运行管理（start/stop/status/logs/restart）不受影响。")
        return 1
    if not src.is_file():
        print(f"❌ 还没有构建产物：{src} 不存在——先 restart（不带 --skip-build）构建一次")
        return 1
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    if BINARY.exists():
        prev = BIN_DIR / "engram-server.prev"
        prev.unlink(missing_ok=True)
        BINARY.rename(prev)  # 现役退为 .prev——新版有问题可一键回滚
    shutil.copy2(src, BINARY)
    # 管理脚本自身也装一份：本机自包含（拔盘后 stop/start/status 仍可用）
    shutil.copy2(Path(__file__).resolve(), BIN_DIR / "engramctl")
    os.chmod(BIN_DIR / "engramctl", 0o755)
    print(f"[install] ✅ 二进制 → {BINARY}")
    print(f"[install]    脚本 → {BIN_DIR / 'engramctl'}")
    return 0


def cmd_start(skip_build: bool) -> int:
    env = load_env()
    dsn = env.get("ENGRAM_DATABASE_URL", "postgres://trtyr@127.0.0.1:5432/engram")

    if pid := server_pid():
        print(f"已在运行（pid {pid}）。要重启用 restart。")
        return 0

    print(f"[1/4] PG 就绪检查（{dsn}）...", end=" ", flush=True)
    if not pg_ready(dsn):
        print("❌ PG 不可达——确认 brew postgresql@16 已启动（brew services start postgresql@16）")
        return 1
    print("✅")

    if not skip_build:
        if rc := build_release():
            return rc
        # 构建产物装进本机运行时家——engram 从 ~/.engram/bin 跑，与代码仓库所在卷解耦
        if rc := cmd_install():
            return rc
    elif BINARY.exists():
        print("[2/4] 跳过 build（--skip-build）✅")
    else:
        print("❌ --skip-build 但本机没有已安装的二进制（~/.engram/bin/engram-server）")
        print("   先跑一次不带 --skip-build 的 start（构建并安装），或回到代码仓库构建后 install")
        return 1

    print("[3/4] 后台挂起 engram-server...", end=" ", flush=True)
    child_env = {
        **os.environ,
        "AGENT_MEMORY_DATABASE_URL": dsn,
        "AGENT_MEMORY_PORT": str(PORT),
        "AGENT_MEMORY_ADMIN_PASSWORD": env.get("AGENT_MEMORY_ADMIN_PASSWORD", "dev-pw"),
        "AGENT_MEMORY_MASTER_KEY": env.get("AGENT_MEMORY_MASTER_KEY", "ab" * 32),
        "AGENT_MEMORY_DATA_DIR": env.get("AGENT_MEMORY_DATA_DIR", str(HOME / ".engram" / "app")),
        "RUST_LOG": env.get("RUST_LOG", "info"),
    }
    if env.get("AGENT_MEMORY_METRICS"):
        child_env["AGENT_MEMORY_METRICS"] = env["AGENT_MEMORY_METRICS"]
    with open(LOG_FILE, "ab") as log:
        proc = subprocess.Popen(
            [str(BINARY)], cwd=RUNTIME_DIR, env=child_env,
            stdout=log, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    PID_FILE.write_text(str(proc.pid))
    print(f"pid {proc.pid}")

    print("[4/4] 健康检查...", end=" ", flush=True)
    for _ in range(30):
        if state := ready_probe():
            print(f"✅ {state}")
            print(f"日志：{LOG_FILE}")
            return 0
        time.sleep(1)
    print(f"❌ 30s 内 /ready 未就绪——查日志：tail -50 {LOG_FILE}")
    return 1


def cmd_stop() -> int:
    pid = server_pid()
    if not pid:
        print("没有在运行的 engram-server。")
        PID_FILE.unlink(missing_ok=True)
        return 0
    print(f"停止 pid {pid}...", end=" ", flush=True)
    os.kill(pid, signal.SIGTERM)
    for _ in range(100):  # 10s
        try:
            os.kill(pid, 0)
            time.sleep(0.1)
        except ProcessLookupError:
            print("✅")
            PID_FILE.unlink(missing_ok=True)
            return 0
    os.kill(pid, signal.SIGKILL)
    print("（SIGKILL）✅")
    PID_FILE.unlink(missing_ok=True)
    return 0


def cmd_restart(skip_build: bool) -> int:
    """重启 = **build 成功 → 才停 + 起**。

    顺序就是这个函数存在的全部理由：先 build，失败就直接返回（正在运行的服务原封不动），
    只有 build 成功才停服务。旧实现反着来（先 stop 再 build），build 一失败服务就下线了。
    """
    if not skip_build:
        print("[restart] build 先行——build 失败则不动正在运行的服务")
        if rc := build_release():
            return rc
        if rc := cmd_install():
            return rc
    elif not BINARY.exists():
        print("❌ --skip-build 但本机没有已安装的二进制（~/.engram/bin/engram-server）")
        print("   先跑一次不带 --skip-build 的 restart（构建并安装）")
        return 1
    cmd_stop()
    time.sleep(1)
    # build 已在上方完成（或按 --skip-build 显式跳过），start 不必重复编译
    return cmd_start(skip_build=True)


def cmd_status() -> int:
    pid = server_pid()
    state = ready_probe()
    if pid and state:
        print(f"运行中 ✅ pid={pid} migration_version={state.get('migration_version')}")
        return 0
    if pid:
        print(f"进程在（pid {pid}）但 /ready 不通——启动中或异常，查日志")
        return 1
    print("未运行。")
    return 1


def cmd_logs(n: int) -> int:
    if not LOG_FILE.exists():
        print("无日志文件。")
        return 1
    lines = LOG_FILE.read_text(errors="replace").splitlines()
    for l in lines[-n:]:
        print(l)
    return 0


def main() -> int:
    args = sys.argv[1:]
    cmd = args[0] if args else "status"
    skip_build = "--skip-build" in args
    if cmd == "start":
        return cmd_start(skip_build)
    if cmd == "stop":
        return cmd_stop()
    if cmd == "restart":
        return cmd_restart(skip_build)
    if cmd == "status":
        return cmd_status()
    if cmd == "install":
        return cmd_install()
    if cmd == "logs":
        n = int(args[1]) if len(args) > 1 else 50
        return cmd_logs(n)
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
