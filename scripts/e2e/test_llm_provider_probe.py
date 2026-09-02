"""E2E #10a：真 provider 注册 + 连通探测（chat + embedding）+ 用量记账。

同时验证 embed 兼容修复：上游（硅基流动 bge-m3）拒绝 dimensions 参数时，
provider 应去掉该参数重试并成功。无 E2E_LLM_API_KEY 时 SKIP（exit 0）。
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client


def _probe_ok(base_url: str, key: str, model: str) -> bool:
    """网关连通预检，避免把网关故障误报成服务缺陷。"""
    try:
        r = rq.post(f"{base_url}/v1/embeddings",
                    headers={"Authorization": f"Bearer {key}"},
                    json={"model": model, "input": ["ping"]}, timeout=30)
        return r.status_code == 200
    except rq.RequestException:
        return False


async def main() -> None:
    e = env.ensure()
    if not e.llm_api_key:
        print("SKIP：未设置 E2E_LLM_API_KEY（LLM 连通测试需要）")
        return
    if not _probe_ok(e.llm_base_url, e.llm_api_key, e.llm_embed_model):
        print(f"SKIP：网关 embedding 预检失败（{e.llm_base_url} {e.llm_embed_model}）——网关侧问题，不算服务缺陷")
        return

    admin = Client.login(e.base_url, e.admin_password)

    section("注册 provider（chat + embedding 各一行）")
    prov = admin.post("/settings/llm/providers", json={
        "name": "e2e-gw",
        "base_url": e.llm_base_url,
        "api_key": e.llm_api_key,
        "model_id": e.llm_chat_model,
        "capability": "chat",
        "is_default": True,
    })
    ok(str(prov.get("id", "")) != "", "provider 创建返回 id")
    pid = prov["id"]
    prov2 = admin.post("/settings/llm/providers", json={
        "name": "e2e-gw-embed",
        "base_url": e.llm_base_url,
        "api_key": e.llm_api_key,
        "model_id": e.llm_embed_model,
        "capability": "embedding",
        "is_default": True,
    })
    pid2 = prov2["id"]

    section("连通探测：chat + embedding")
    r1 = admin.post(f"/settings/llm/providers/{pid}/test", timeout=120)
    eq(r1.get("ok"), True, f"chat 探测 ok=true（{r1.get('message', '')}）")
    r2 = admin.post(f"/settings/llm/providers/{pid2}/test", timeout=120)
    eq(r2.get("ok"), True, f"embed 探测 ok=true（{r2.get('message', '')}）")

    section("用量记账出现")
    usage = admin.get("/llm/usage")
    ok(isinstance(usage, list) and len(usage) >= 1, "llm_usage 有记录")
    if usage:
        row = usage[0]
        ok("provider" in row and "purpose" in row, f"记账行含 provider/purpose（{row.get('provider')}/{row.get('purpose')}）")

    section("key 不回显（安全）")
    provs = admin.get("/settings/llm/providers")
    p = next(x for x in provs if x["name"] == "e2e-gw")
    ok("api_key" not in str(p), "provider 列表不回显明文 key")


check.run(main)
