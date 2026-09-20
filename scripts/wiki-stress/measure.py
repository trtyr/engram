#!/usr/bin/env python3
"""wiki 规模化基线测量——对压测实例测目录树/图谱/检索/重查询延迟。

用法:
    python3 measure.py --label baseline-1k
    python3 measure.py --label after-10k --base http://127.0.0.1:17660

输出: scripts/wiki-stress/results/<label>.json + 控制台摘要
指标: 每项跑 RUNS 次取中位（insights 重查询只跑 1 次）。
"""
import argparse
import json
import statistics
import time
import urllib.request
from pathlib import Path

RUNS = 3


def req(base: str, token: str, path: str, body: dict | None = None, timeout: int = 120) -> tuple[float, int, bytes]:
    url = f"{base}{path}"
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method="POST" if body is not None else "GET")
    r.add_header("Authorization", f"Bearer {token}")
    if body is not None:
        r.add_header("Content-Type", "application/json")
    t0 = time.perf_counter()
    with urllib.request.urlopen(r, timeout=timeout) as resp:
        payload = resp.read()
    return (time.perf_counter() - t0) * 1000, resp.status, payload


def median_ms(runs: list[float]) -> float:
    return round(statistics.median(runs), 1)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:17660")
    ap.add_argument("--label", required=True)
    ap.add_argument("--user", default="admin")
    ap.add_argument("--password", default="stress-test-pw")
    args = ap.parse_args()

    # 登录
    t0 = time.perf_counter()
    r = urllib.request.Request(
        f"{args.base}/auth/login",
        data=json.dumps({"username": args.user, "password": args.password}).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(r, timeout=10) as resp:
        token = json.load(resp)["token"]
    print(f"登录耗时 {(time.perf_counter() - t0) * 1000:.0f}ms")

    report: dict = {"label": args.label, "base": args.base, "measured_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "metrics": {}}
    m = report["metrics"]

    # 1) 目录树全量
    times, size, count = [], 0, 0
    for _ in range(RUNS):
        ms, _, payload = req(args.base, token, "/wiki/pages")
        times.append(ms)
        size = len(payload)
        count = len(json.loads(payload))
    m["pages_list"] = {"median_ms": median_ms(times), "bytes": size, "count": count, "runs": [round(t, 1) for t in times]}
    print(f"目录树全量: {median_ms(times)}ms（{count} 页, {size // 1024}KB）")

    # 2) 图谱接口
    times = []
    nodes = edges = 0
    for _ in range(RUNS):
        ms, _, payload = req(args.base, token, "/wiki/graph", timeout=300)
        times.append(ms)
        g = json.loads(payload)
        nodes, edges = len(g.get("nodes", [])), len(g.get("edges", []))
    m["graph_api"] = {"median_ms": median_ms(times), "nodes": nodes, "edges": edges, "runs": [round(t, 1) for t in times]}
    print(f"图谱接口: {median_ms(times)}ms（{nodes} 节点 / {edges} 边）")

    # 3) 检索 P50/P95——20 个真实词（从页面 title 抽取）
    pages = json.loads(req(args.base, token, "/wiki/pages")[2])
    import re

    words: list[str] = []
    for p in pages:
        for w in re.findall(r"[\u4e00-\u9fa5]{2,6}|[a-z]{3,}", p.get("title", "")):
            if w not in words:
                words.append(w)
        if len(words) >= 40:
            break
    sample = words[:40:2]  # 20 个
    lat = []
    zero_hits = 0
    for w in sample:
        ms, _, payload = req(args.base, token, "/wiki/search", {"query": w, "max_items": 10})
        lat.append(ms)
        hits = json.loads(payload).get("pages", [])
        if not hits:
            zero_hits += 1
    lat.sort()
    m["search"] = {
        "p50_ms": round(statistics.median(lat), 1),
        "p95_ms": round(lat[int(len(lat) * 0.95) - 1], 1),
        "max_ms": round(lat[-1], 1),
        "zero_hits": zero_hits,
        "queries": len(sample),
    }
    print(f"检索: P50 {m['search']['p50_ms']}ms / P95 {m['search']['p95_ms']}ms / max {m['search']['max_ms']}ms（{len(sample)} 查询, 零命中 {zero_hits}）")

    # 4) 重查询：insights（跑 1 次）
    try:
        ms, _, _ = req(args.base, token, "/wiki/insights", {}, timeout=600)
        m["insights"] = {"ms": round(ms, 1)}
        print(f"图谱洞察: {ms:.0f}ms")
    except Exception as e:  # noqa: BLE001
        m["insights"] = {"error": str(e)[:200]}
        print(f"图谱洞察: 失败 {e}")

    # 5) 缺口清单
    ms, _, _ = req(args.base, token, "/wiki/query-gaps")
    m["query_gaps"] = {"ms": round(ms, 1)}
    print(f"缺口清单: {ms:.0f}ms")

    out = Path(__file__).parent / "results"
    out.mkdir(exist_ok=True)
    f = out / f"{args.label}.json"
    f.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"报告已存: {f}")


if __name__ == "__main__":
    main()
