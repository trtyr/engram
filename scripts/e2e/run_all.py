#!/usr/bin/env python3
"""顺序跑全部 test_*.py（每个独立进程 = 一个测试项），汇总结果。

用法：
    python3 run_all.py            # 全部
    python3 run_all.py test_health_auth.py test_apikeys_scopes.py   # 指定
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> int:
    args = sys.argv[1:]
    if args:
        tests = [HERE / a for a in args]
    else:
        tests = sorted(HERE.glob("test_*.py"))
    if not tests:
        print("没有可运行的 test_*.py")
        return 1

    results: list[tuple[str, str]] = []
    for t in tests:
        print(f"\n━━━━━━━━━━━━━━━━━━━━ {t.name} ━━━━━━━━━━━━━━━━━━━━", flush=True)
        p = subprocess.run([sys.executable, str(t)], cwd=HERE)
        results.append((t.name, "PASS" if p.returncode == 0 else f"FAIL({p.returncode})"))

    print("\n━━━━━━━━━━━━━━ 汇总 ━━━━━━━━━━━━━━")
    for name, r in results:
        print(f"  {r:<10} {name}")
    fails = [n for n, r in results if r != "PASS"]
    print(f"\n{'✗ ' + str(len(fails)) + ' 项失败' if fails else '✓ 全部通过'}（共 {len(results)} 项）")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
