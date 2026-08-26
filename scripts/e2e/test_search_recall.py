"""E2E #11：召回兜底——长查询 OR 语义不零命中，短查询保持 AND 精确（R2 行为验证）。

纯确定性：无 provider（FTS-only），种子原子只含 Rust 关键词。
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import Client


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    mem = admin.with_key(admin.create_api_key("e2e-recall", ["memory"]))

    section("种子：只含 Rust 的原子")
    mem.post("/memory/atoms", json={
        "kind": "fact", "content": "用户偏好使用 Rust 语言进行系统编程", "confidence": 0.95,
    })

    section("长查询（7 token > 3）→ OR 兜底，部分命中即返回")
    res = mem.post("/memory/search", json={"query": "上海 北京 烤鸭 旅行 电影 音乐 rust"})
    ok(len(res["l1"]) >= 1,
       f"部分 token 命中即返回（{len(res['l1'])}）——旧行为是全 AND 零命中")

    section("短查询（2 token ≤ 3）→ AND 精确，不含关键词则零命中")
    res2 = mem.post("/memory/search", json={"query": "上海 烹饪"})
    eq(res2["l1"], [], "不相关短查询零命中（AND 精确语义保留）")

    section("控制组：短查询含关键词 → 命中")
    res3 = mem.post("/memory/search", json={"query": "Rust 编程"})
    ok(len(res3["l1"]) >= 1, "相关短查询正常命中")


check.run(main)
