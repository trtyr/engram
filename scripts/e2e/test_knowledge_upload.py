"""E2E #5：知识摄取全链路——上传 → parse→chunk→embed → ready → 检索 → 幂等 → 删除。

有 LLM 凭证时注册真 provider（断言嵌入成功）；无则断言降级路径（embed_failed 仍 ready）。
"""

import io
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import requests as rq

from _lib import check, env
from _lib.check import eq, ok, section
from _lib.client import ApiError, Client

DOC = """# pgvector 混合检索指南

pgvector 是 PostgreSQL 的向量检索扩展，支持 HNSW 索引与余弦距离。
本项目用它存储 1024 维嵌入，并用 RRF 融合全文与向量两路召回。

## 中文分词

写入 tsvector 前用 jieba 预分词，保证写入与查询同源。
单汉字信息量低会被过滤，只保留两字以上的词。

## 召回兜底

长查询走 OR 语义，避免全 AND 导致零命中；短查询保持 AND 精确。
""".encode("utf-8")


def _embed_gw_ok(e) -> bool:
    if not e.llm_api_key:
        return False
    try:
        r = rq.post(f"{e.llm_base_url}/v1/embeddings",
                    headers={"Authorization": f"Bearer {e.llm_api_key}"},
                    json={"model": e.llm_embed_model, "input": ["ping"]}, timeout=30)
        return r.status_code == 200
    except rq.RequestException:
        return False


async def main() -> None:
    e = env.ensure()
    admin = Client.login(e.base_url, e.admin_password)
    has_llm = _embed_gw_ok(e)
    if has_llm:
        admin.post("/settings/llm/providers", json={
            "name": "e2e-know", "base_url": e.llm_base_url, "api_key": e.llm_api_key,
            "model_id": e.llm_embed_model, "capability": "embedding", "is_default": True,
        })
    know = admin.with_key(admin.create_api_key("e2e-know", ["knowledge"]))
    print(f"[info] LLM embedding: {'真网关' if has_llm else '无（验证降级路径）'}")

    section("上传 markdown")
    r = know.s.post(f"{know.base}/knowledge/upload",
                    files={"file": ("pgvector-guide.md", io.BytesIO(DOC), "text/markdown")},
                    timeout=30)
    eq(r.status_code, 201, "首次上传 201")
    doc = r.json()
    doc_id = doc["id"]
    eq(doc["status"], "pending", "初始状态 pending")

    section("等状态机走到 ready")
    deadline = time.time() + 120
    while time.time() < deadline:
        doc = know.get(f"/knowledge/documents/{doc_id}")
        if doc["status"] in ("ready", "failed"):
            break
        time.sleep(1)
    eq(doc["status"], "ready", f"终态 ready（{doc.get('error') or '无错误'}）")

    section("chunks 采样")
    chunks = know.get(f"/knowledge/documents/{doc_id}/chunks")
    ok(len(chunks) >= 1, f"chunk 数 ≥1（实际 {len(chunks)}）")
    joined = " ".join(c["content"] for c in chunks)
    ok("pgvector" in joined, "chunk 内容含关键主题")
    if has_llm:
        ok(all(not c["embed_failed"] for c in chunks), "全部 chunk 嵌入成功")
    else:
        ok(all(c["embed_failed"] for c in chunks), "无 provider 时全部降级 embed_failed（不阻塞 ready）")

    section("检索命中（带文档引用）")
    hits = know.post("/knowledge/search", json={"query": "pgvector 向量检索"})
    ok(len(hits) >= 1, f"检索命中（{len(hits)}）")
    ok(any(h["document_title"] == "pgvector-guide.md" for h in hits), "命中带文档标题引用")

    section("幂等：重复上传同内容 → 200 复用同一文档")
    r2 = know.s.post(f"{know.base}/knowledge/upload",
                     files={"file": ("pgvector-guide.md", io.BytesIO(DOC), "text/markdown")},
                     timeout=30)
    eq(r2.status_code, 200, "重复上传 200（dedup）")
    eq(r2.json()["id"], doc_id, "返回同一文档 id")

    section("删除级联")
    know.delete(f"/knowledge/documents/{doc_id}")
    try:
        know.get(f"/knowledge/documents/{doc_id}")
        raise check.Fail("应 404")
    except ApiError as err:
        eq(err.status, 404, "删除后 GET 404")


check.run(main)
