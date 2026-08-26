"""断言辅助：小而吵——失败打印完整上下文并带退出码退出。"""

from __future__ import annotations

import sys
import traceback

_PASSES = 0


class Fail(Exception):
    pass


def section(name: str) -> None:
    print(f"\n=== {name} ===", flush=True)


def ok(cond: bool, msg: str, ctx: Any = None) -> None:
    global _PASSES
    if not cond:
        raise Fail(f"{msg}" + (f"\n  上下文: {ctx!r}" if ctx is not None else ""))
    _PASSES += 1
    print(f"  ✓ {msg}", flush=True)


def eq(actual, expected, msg: str) -> None:
    ok(actual == expected, f"{msg}（期望 {expected!r}，实际 {actual!r}）")


def run(main) -> None:
    """test_*.py 统一入口：python test_xxx.py → exit 0/1。

    main 是 async 或 sync 函数；异常时打印服务日志尾部 + 完整 traceback。
    """
    import asyncio

    name = sys.argv[0]
    print(f"▶ {name}", flush=True)
    try:
        coro = main()
        if asyncio.iscoroutine(coro):
            asyncio.run(coro)
        else:
            coro
    except Exception as e:  # noqa: BLE001
        from _lib import env as _env

        print(f"\n✗ 失败：{e}", flush=True)
        try:
            current = _env._ENV  # noqa: SLF001 — 排障取当前环境
            if current:
                print(f"\n--- 服务日志尾部 ---\n{current.tail_log()}", flush=True)
        except Exception:  # noqa: BLE001
            pass
        traceback.print_exc()
        sys.exit(1)
    print(f"\n✓ 全部通过（{_PASSES} 项断言）", flush=True)
