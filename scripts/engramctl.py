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
  python3 scripts/engramctl.py sync push|pull <目标地址> [--token <migrate_key>]
                                      [--local <本机实例地址>] [--dry-run] [--allow-insecure]
                                                        # 数据同步（手动触发，非定期）：
                                                        # push=本地→目标；pull=目标→本地。
                                                        # 目标侧凭证用 migrate scope 的 API key
                                                        # （--token 或环境变量 ENGRAM_SYNC_TOKEN）——
                                                        # admin 密码不过公网；本机侧自动用本地 admin。
                                                        # --local 覆盖本机侧地址（默认
                                                        # http://127.0.0.1:17654；对临时测试实例
                                                        # 做 pull 验收时指向它）。
                                                        # 非 loopback 目标强制 https（http 拒绝，
                                                        # --allow-insecure 逃生）；ssh 隧道场景：
                                                        # ssh -L 17654:127.0.0.1:17654 user@cloud
                                                        # 后目标填 http://127.0.0.1:17654

    python3 scripts/engramctl.py codegraph push|pull <目标地址> --project <项目名>
                                        [--path <本地仓库>] [--token <codegraph_key>]
                                        [--local <本机实例>] [--dry-run] [--allow-insecure]
                                                          # codegraph 产物同步（手动触发，非定期）：
                                                          # push=本地产物→目标；pull=目标产物→本地。
                                                          # 目标侧凭证用 codegraph scope 的 API key
                                                          # （--token 或 ENGRAM_CODEGRAPH_TOKEN；
                                                          # admin 密码不过公网）。push 走目标 MCP
                                                          # codegraph.upload；pull 走目标产物下载端点
                                                          # 后按同一份声明投给本地实例（等价一次 upload）。
                                                          # 安全口径与 sync 同（非 loopback 强制 https）

配置源：~/.engram/.env（KEY=VALUE）——需含：
  AGENT_MEMORY_ADMIN_PASSWORD / AGENT_MEMORY_MASTER_KEY
  ENGRAM_DATABASE_URL（宿主 PG 连接串，如 postgres://trtyr@127.0.0.1:5432/engram）
可选：AGENT_MEMORY_METRICS、RUST_LOG（默认 info）
"""

from __future__ import annotations  # 兼容 3.9 解析 `int | None` 注解（注解惰性化）

import json
import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.parse
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
    # EN-244：HTTP API 手册随部署分发（在线端点 GET /openapi.json + 本文件双通道）
    dump = src.parent / "openapi-dump"
    if dump.is_file():
        r = subprocess.run([str(dump)], capture_output=True, text=True)
        if r.returncode == 0 and r.stdout.strip():
            (BIN_DIR / "openapi.json").write_text(r.stdout)
            print(f"[install]    API 手册 → {BIN_DIR / 'openapi.json'}")
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


# ---------------------------------------------------------------------------
# 数据同步（2026-09-18 数据同步线）：sync push|pull <目标地址>——手动触发，非定期
# ---------------------------------------------------------------------------


def _sync_check_target(target: str, allow_insecure: bool) -> str | None:
    """目标地址安全口径：非 loopback 强制 https，http 明文拒绝（--allow-insecure 逃生）。

    无 scheme 时默认按 https 解析（宁可安全侧失败）。
    """
    raw = target if "://" in target else f"https://{target}"
    parsed = urllib.parse.urlparse(raw)
    scheme, host = parsed.scheme.lower(), parsed.hostname or ""
    loopback = host in ("127.0.0.1", "localhost", "::1")
    if scheme != "https" and not loopback and not allow_insecure:
        print(f"❌ 目标 {target} 非 loopback 且为 http 明文——公网同步必须走 TLS（迁移安全同口径）")
        print("   方式一：目标用 https:// 域名（反代/TLS 终结后）")
        print("   方式二：ssh 隧道后用 loopback——ssh -L 17654:127.0.0.1:17654 user@cloud，")
        print("           目标填 http://127.0.0.1:17654")
        print("   （本地试跑确要明文：加 --allow-insecure）")
        return None
    return target.rstrip("/")


def _http_json(
    url: str, method: str, token: str, payload: dict | None = None, timeout: int = 600
) -> tuple[int, dict]:
    """极简 JSON API 客户端（纯标准库）：返回 (status, body)。网络错误按 (0, {}) 处理。"""
    req = urllib.request.Request(url, method=method.upper())
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    data = None
    if payload is not None:
        req.add_header("Content-Type", "application/json")
        data = json.dumps(payload).encode()
    try:
        with urllib.request.urlopen(req, data=data, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode() or "{}")
        except Exception:
            return e.code, {}
    except Exception as e:
        print(f"❌ 请求失败 {method} {url}: {e}")
        return 0, {}


def _local_admin_token(base: str) -> str | None:
    """本机侧 admin 凭证登录（默认 loopback 生产实例；--local 可指向临时测试实例）。"""
    pw = load_env().get("AGENT_MEMORY_ADMIN_PASSWORD")
    if not pw:
        print("❌ 本机 ~/.engram/.env 缺 AGENT_MEMORY_ADMIN_PASSWORD")
        return None
    st, v = _http_json(f"{base}/auth/login", "POST", "", {"password": pw}, timeout=15)
    if st != 200 or not v.get("token"):
        print(f"❌ 本机登录失败（HTTP {st}，{base}）——服务在跑吗？python3 scripts/engramctl.py status")
        return None
    return v["token"]


def _print_bundle_summary(tag: str, bundle: dict) -> None:
    counts = bundle.get("counts", {})
    if not counts:
        print(f"[{tag}] ⚠️ 迁移包无 counts 字段：{list(bundle)[:6]}…")
        return
    print(f"[{tag}] 迁移包分域行数（exported_at={bundle.get('exported_at', '?')}）：")
    for k in sorted(counts):
        print(f"    {k}: {counts[k]}")


def _print_import_report(tag: str, report: dict) -> None:
    """导入报告递归打印（分域 imported/skipped）。"""
    print(f"[{tag}] 导入报告（+imported / 跳过 skipped）：")

    def walk(prefix: str, node: dict) -> None:
        for k in sorted(node):
            v = node[k]
            if isinstance(v, dict):
                if "imported" in v:
                    print(f"    {prefix}{k}: +{v.get('imported', 0)} / 跳过 {v.get('skipped', 0)}")
                else:
                    walk(f"{prefix}{k}.", v)

    walk("", report)


def cmd_sync(args: list[str]) -> int:
    """数据同步（手动触发，非定期）：push=本地→目标（正向迁移）；pull=目标→本地（反向回拉）。

    凭证：目标侧 migrate scope 的 API key（--token / ENGRAM_SYNC_TOKEN；admin 密码不过公网），
         本机侧本地 admin（~/.engram/.env 自动登录，仅 loopback）。
    语义：merge 先写为准（冲突跳过）——重跑安全；方向敲反顶多无效果，不毁数据。
    注意：embedding/LLM provider/账号均不随迁（见《上云迁移清单 2026-09-18》）。
    """
    usage = (
        "用法：engramctl sync push|pull <目标地址> [--token <migrate_key>] [--dry-run] [--allow-insecure]"
    )
    if len(args) < 2 or args[0] not in ("push", "pull"):
        print(usage)
        return 2
    direction, target = args[0], args[1]
    dry_run = "--dry-run" in args
    allow_insecure = "--allow-insecure" in args
    token: str | None = None
    if "--token" in args:
        i = args.index("--token")
        if i + 1 >= len(args):
            print("❌ --token 后面要跟 key 值")
            return 2
        token = args[i + 1]
    if token is None:
        token = os.environ.get("ENGRAM_SYNC_TOKEN")
    if not token:
        print("❌ 缺目标侧凭证——用 migrate scope 的 API key（目标机 /account 页创建）")
        print("   传入：--token <key> 或环境变量 ENGRAM_SYNC_TOKEN（不要拿 admin 密码过公网）")
        return 2

    base = _sync_check_target(target, allow_insecure)
    if base is None:
        return 1

    local = f"http://127.0.0.1:{PORT}"
    if "--local" in args:
        i = args.index("--local")
        if i + 1 >= len(args):
            print("❌ --local 后面要跟本机实例地址")
            return 2
        local = args[i + 1].rstrip("/")
    local_token = _local_admin_token(local)
    if local_token is None:
        return 1

    src_label, dst_label = ("本地", "目标") if direction == "push" else ("目标", "本地")
    src_base, src_tok = (local, local_token) if direction == "push" else (base, token)
    dst_base, dst_tok = (base, token) if direction == "push" else (local, local_token)

    print(f"[sync {direction}] ① 导出：{src_label} GET /migrate/export ...")
    st, bundle = _http_json(f"{src_base}/migrate/export", "GET", src_tok)
    if st != 200:
        print(f"❌ {src_label}导出失败（HTTP {st}）：{json.dumps(bundle, ensure_ascii=False)[:300]}")
        return 1
    _print_bundle_summary(src_label, bundle)

    if dry_run:
        print(f"[sync {direction}] --dry-run：不写入{dst_label}。以上为将同步的内容。")
        return 0

    print(f"[sync {direction}] ② 导入：{dst_label} POST /migrate/import ...")
    st, report = _http_json(f"{dst_base}/migrate/import", "POST", dst_tok, payload=bundle)
    if st != 200:
        print(f"❌ {dst_label}导入失败（HTTP {st}）：{json.dumps(report, ensure_ascii=False)[:300]}")
        return 1
    _print_import_report(dst_label, report)
    print(f"[sync {direction}] ✅ 完成。提醒：embedding/LLM provider/账号不随迁（清单见《上云迁移清单 2026-09-18》）。")
    return 0


def _mcp_call(
    base: str, token: str, tool: str, arguments: dict, timeout: int = 1800
) -> tuple[int, dict]:
    """MCP JSON-RPC tools/call（纯标准库）：返回 (status, body)。

    传输是 Streamable HTTP：响应可能是 JSON，也可能是 text/event-stream（每行 `data: {...}`）。
    两种都要吃下——否则会「调用成功却解析不出结果」。Accept 头也必须声明两种，否则 406。
    """
    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments},
    }
    req = urllib.request.Request(f"{base}/mcp", method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Accept", "application/json, text/event-stream")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    data = json.dumps(payload).encode()
    try:
        with urllib.request.urlopen(req, data=data, timeout=timeout) as resp:
            raw, ctype, status = resp.read().decode(), resp.headers.get("Content-Type", ""), resp.status
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode() or "{}")
        except Exception:
            return e.code, {}
    except Exception as e:
        print(f"❌ MCP 调用失败 {base}/mcp: {e}")
        return 0, {}
    if "text/event-stream" in ctype:
        for line in raw.splitlines():
            if line.startswith("data:"):
                try:
                    return status, json.loads(line[5:].strip())
                except Exception:
                    continue
        return status, {}
    try:
        return status, json.loads(raw or "{}")
    except Exception:
        return status, {}


def _mcp_json_result(body: dict) -> dict:
    """取 tools/call 里的业务 JSON 载荷（structuredContent 或 content[].text）。"""
    result = body.get("result") or {}
    sc = result.get("structuredContent")
    if isinstance(sc, dict) and sc:
        return sc
    for item in result.get("content") or []:
        if isinstance(item, dict) and item.get("type") == "text":
            try:
                return json.loads(item.get("text") or "{}")
            except Exception:
                continue
    return {}


def _mcp_ok(body: dict) -> tuple[bool, str]:
    """tools/call 结果判定：JSON-RPC error / isError 都算失败。"""
    if body.get("error"):
        return False, json.dumps(body["error"], ensure_ascii=False)
    result = body.get("result") or {}
    if result.get("isError"):
        content = result.get("content") or []
        text = content[0].get("text") if content and isinstance(content[0], dict) else ""
        return False, text or "工具返回 isError"
    return True, ""


def _http_bytes(url: str, token: str, timeout: int = 1800) -> tuple[int, bytes, dict]:
    """原始字节 GET（产物下载用）：返回 (status, bytes, headers)。"""
    req = urllib.request.Request(url, method="GET")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read(), dict(resp.headers)
    except urllib.error.HTTPError as e:
        return e.code, b"", dict(e.headers or {})
    except Exception as e:
        print(f"❌ 请求失败 GET {url}: {e}")
        return 0, b"", {}


def _codegraph_local_db(repo: str) -> tuple[str, str]:
    """定位本地仓库的产物与 HEAD：返回 (db 路径, head)。

    仓库根由 `git rev-parse --show-toplevel` 反查——CLI 的产物落在**仓库根**的
    `.codegraph/codegraph.db`（子目录注册也命中同一个产物；与 cg-bridge 的 index_db_path 同口径）。
    """
    try:
        root = subprocess.run(
            ["git", "-C", repo, "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        head = subprocess.run(
            ["git", "-C", repo, "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
    except Exception as e:
        print(f"❌ {repo} 不是 git 仓库或 git 不可用: {e}")
        return "", ""
    return os.path.join(root, ".codegraph", "codegraph.db"), head


def cmd_codegraph(args: list[str]) -> int:
    """codegraph 产物同步（手动触发，非定期）：push=本地产物→目标；pull=目标产物→本地。

    凭证：目标侧 codegraph scope 的 API key（--token / ENGRAM_CODEGRAPH_TOKEN；admin 密码不过公网）；
         本机侧本地 admin（~/.engram/.env 自动登录，仅 loopback）。
    通道：push 走目标 MCP `codegraph.upload`（base64，与 pi 侧扩展同通道）；pull 走目标
         `GET /codegraph/projects/{id}/artifact`（原始字节 + 元数据响应头），拿到后按**同一份声明**
         投给本地实例（等价于一次 upload）——两个方向都不新增服务端组件。
    安全口径：与 sync 同（非 loopback 强制 https；--allow-insecure 逃生；ssh 隧道写法同前）。
    """
    usage = (
        "用法：engramctl codegraph push|pull <目标地址> --project <项目名> "
        "[--path <本地仓库>] [--token <codegraph_key>] [--local <本机实例>] "
        "[--dry-run] [--allow-insecure]"
    )
    if len(args) < 2 or args[0] not in ("push", "pull"):
        print(usage)
        return 2
    direction, target = args[0], args[1]

    def opt(flag: str, default: str | None = None) -> str | None:
        if flag in args:
            i = args.index(flag)
            if i + 1 >= len(args):
                print(f"❌ {flag} 后面要跟值")
                return None
            return args[i + 1]
        return default

    project = opt("--project")
    if not project:
        print("❌ 缺 --project <项目名>（两侧同名；upload 通道的 name 即项目名）")
        return 2
    path = opt("--path", os.getcwd()) or os.getcwd()
    local = opt("--local", f"http://127.0.0.1:{PORT}") or f"http://127.0.0.1:{PORT}"
    dry_run = "--dry-run" in args
    token = opt("--token") or os.environ.get("ENGRAM_CODEGRAPH_TOKEN")
    if not token:
        print("❌ 缺目标侧凭证——用 codegraph scope 的 API key（目标机 /account 页创建）")
        print("   传入：--token <key> 或环境变量 ENGRAM_CODEGRAPH_TOKEN（不要拿 admin 密码过公网）")
        return 2

    base = _sync_check_target(target, "--allow-insecure" in args)
    if base is None:
        return 1

    if direction == "push":
        db, head = _codegraph_local_db(path)
        if not db:
            return 1
        if not os.path.isfile(db):
            print(f"❌ 本地产物不存在：{db}")
            print("   先在本机对 {path} 跑 `codegraph index`（首建用 init）生成产物再推")
            return 1
        size = os.path.getsize(db)
        print(f"[push] 项目 {project} → {base}")
        print(f"       产物 {db}（{size / 1024 / 1024:.1f} MB）head {head[:12]}")
        if dry_run:
            print("       --dry-run：只探明来源，不发起推送 ✅")
            return 0
        # 大 body 前的轻量鉴权预检：服务端在读写完 body 之前就拒（401/403）时，客户端只会看到
        # 连接重置（写 body 撞上对端关闭）——先探一次，失败路径文案才可行动
        st_probe, _ = _http_json(f"{base}/codegraph/projects", "GET", token, timeout=30)
        if st_probe in (401, 403):
            print(f"❌ 目标侧凭证被拒（HTTP {st_probe}）——codegraph scope 的 API key 无效或已吊销")
            print("   到目标机 /account 页重建一把带 codegraph scope 的 key，或确认 --token 是不是那把")
            return 1
        if st_probe != 200:
            print(f"❌ 目标 codegraph 面不可达（HTTP {st_probe}）——先 curl {base}/health 确认服务正常")
            return 1
        import base64 as _b64

        with open(db, "rb") as f:
            db_b64 = _b64.b64encode(f.read()).decode()
        st, body = _mcp_call(
            base,
            token,
            "codegraph",
            {"action": "upload", "name": project, "head": head, "db_b64": db_b64},
        )
        ok, why = _mcp_ok(body)
        if st not in (200, 202) or not ok:
            print(f"❌ 推送失败（HTTP {st}）：{why or json.dumps(body, ensure_ascii=False)[:400]}")
            return 1
        res = _mcp_json_result(body)
        print(
            f"       ✅ 已推送：status={res.get('status', '?')} "
            f"source_kind={res.get('source_kind', '?')} "
            f"built_with_version={res.get('built_with_version', '?')}"
        )
        return 0

    # pull：远端产物 → 本地实例（按同一份声明投本地，等价一次 upload）
    st, projects = _http_json(f"{base}/codegraph/projects", "GET", token, timeout=60)
    if st in (401, 403):
        print(f"❌ 目标侧凭证被拒（HTTP {st}）——codegraph scope 的 API key 无效或已吊销")
        print("   到目标机 /account 页重建一把带 codegraph scope 的 key，或确认 --token 是不是那把")
        return 1
    if st != 200:
        print(f"❌ 读目标项目列表失败（HTTP {st}）")
        return 1
    row = next((p for p in projects if p.get("name") == project), None)
    if row is None:
        names = "、".join(p.get("name", "?") for p in projects) or "（空）"
        print(f"❌ 目标没有项目 {project}——已注册：{names}")
        return 1
    st, blob, headers = _http_bytes(f"{base}/codegraph/projects/{row['id']}/artifact", token)
    if st != 200:
        print(f"❌ 下载产物失败（HTTP {st}）：{blob[:200].decode(errors='replace')}")
        return 1
    rhead = headers.get("x-codegraph-head", "")
    print(f"[pull] {base} 项目 {project} → 本地 {local}")
    print(
        f"       产物 {len(blob) / 1024 / 1024:.1f} MB head {rhead[:12] or '（未声明）'} "
        f"source_kind={headers.get('x-codegraph-source-kind', '?')}"
    )
    if not rhead:
        print("❌ 远端该条目没有 head 声明——upload 通道要求 head，无法原样投给本地")
        print("   （云端自建条目请先在目标机 index/sync，让它带上 head 再 pull）")
        return 1
    if dry_run:
        print("       --dry-run：只探明来源，不发起落库 ✅")
        return 0
    local_token = _local_admin_token(local)
    if local_token is None:
        return 1
    import base64 as _b64

    st, body = _mcp_call(
        local,
        local_token,
        "codegraph",
        {
            "action": "upload",
            "name": project,
            "head": rhead,
            "db_b64": _b64.b64encode(blob).decode(),
        },
    )
    ok, why = _mcp_ok(body)
    if st not in (200, 202) or not ok:
        print(f"❌ 落库失败（HTTP {st}）：{why or json.dumps(body, ensure_ascii=False)[:400]}")
        return 1
    res = _mcp_json_result(body)
    print(
        f"       ✅ 已落库到本地：status={res.get('status', '?')} "
        f"source_kind={res.get('source_kind', '?')}"
    )
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
    if cmd == "sync":
        return cmd_sync(args[1:])
    if cmd == "codegraph":
        return cmd_codegraph(args[1:])
    if cmd == "logs":
        n = int(args[1]) if len(args) > 1 else 50
        return cmd_logs(n)
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
