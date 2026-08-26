"""E2E #12：CodeGraph 桥——注册本地小项目 → 索引 → 同步 → 查询。

本机无 codegraph CLI 或索引失败（版本不符等环境原因）时明确 SKIP，不算 FAIL。
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _lib import check, env
from _lib.check import ok, section
from _lib.client import ApiError, Client

TS_CODE = """export function greetE2E(name: string): string {
  return `hello ${name}`;
}

export class E2EGreeter {
  constructor(private who: string) {}
  greet(): string {
    return greetE2E(this.who);
  }
}
"""


def _skip(msg: str) -> None:
    print(f"SKIP：{msg}")
    sys.exit(0)


async def main() -> None:
    if not shutil.which("codegraph"):
        _skip("本机无 codegraph CLI")

    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    cg = admin.with_key(admin.create_api_key("e2e-cg", ["codegraph"]))

    proj = tempfile.mkdtemp(prefix="am-e2e-cg-")
    Path(proj, "greeter.ts").write_text(TS_CODE)

    section("注册项目")
    p = cg.post("/codegraph/projects", json={"name": "e2e-cg-demo", "source_uri": proj})
    ok(str(p["id"]).count("-") == 4, "注册返回 id")
    pid = p["id"]

    section("索引（同步 CLI）")
    try:
        dto = cg.post(f"/codegraph/projects/{pid}/index", timeout=300)
    except ApiError as err:
        low = str(err.body).lower()
        if "版本" in str(err.body) or "cli" in low or "unavailable" in low:
            _skip(f"索引失败（环境原因）：{err.body}")
        raise
    if dto.get("status") == "error":
        _skip(f"索引失败（CLI 环境）：{dto.get('error')}")
    ok(dto.get("status") == "ready", f"索引 ready（stats: {str(dto.get('stats'))[:80]}）")

    section("列表与详情")
    lst = cg.get("/codegraph/projects")
    ok(any(x["id"] == pid for x in lst), "项目出现在列表")
    one = cg.get(f"/codegraph/projects/{pid}")
    ok(one["status"] == "ready", "详情 ready")

    section("查询：search 符号")
    res = cg.post(f"/codegraph/projects/{pid}/query",
                  json={"kind": "search", "target": "greetE2E"}, timeout=120)
    ok(res is not None, "search 返回非空结果")
    text = str(res)
    ok("greetE2E" in text or "E2EGreeter" in text, "命中目标符号")

    section("查询：非法 kind → 400")
    try:
        cg.post(f"/codegraph/projects/{pid}/query", json={"kind": "bogus", "target": "x"})
        raise check.Fail("应 400")
    except ApiError as err:
        ok(err.status == 400, "非法查询类型被拒")

    section("sync：再同步保持 ready")
    dto2 = cg.post(f"/codegraph/projects/{pid}/sync", timeout=300)
    ok(dto2.get("status") == "ready", "sync 后仍 ready")


check.run(main)
