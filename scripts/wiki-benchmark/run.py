#!/usr/bin/env python3
"""wiki 检索基准集评分脚本（批次⑥）——改检索必跑，降级立刻可见。

用法：
  python3 scripts/wiki-benchmark/run.py                # 跑基准，打印报告
  python3 scripts/wiki-benchmark/run.py --out out.json # 报告另存 JSON
  python3 scripts/wiki-benchmark/run.py --label v2     # 报告带标签（对比用）

指标：Hit@5 / Hit@10（gold 任一命中）/ MRR@5（主 gold 或任一 gold 首次命中排名倒数）。
"""
import argparse
import json
import os
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = os.environ.get("ENGRAM_BASE", "http://127.0.0.1:17654")
K5, K10 = 5, 10


def http(method, path, body=None, token=None):
    req = urllib.request.Request(BASE + path, method=method)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, data) as r:
            return r.status, json.loads(r.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        raise SystemExit(f"HTTP {e.code} on {path}: {e.read().decode()[:300]}")


def login():
    env = {}
    for line in open(os.path.expanduser("~/.engram/.env")):
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            env[k] = v
    st, resp = http("POST", "/auth/login", {"username": "admin", "password": env["AGENT_MEMORY_ADMIN_PASSWORD"]})
    assert st == 200, "login failed"
    return resp["token"]


def score_case(tok, case, latencies, rerank=False):
    body = {"query": case["q"], "max_items": K10}
    if rerank:
        body["rerank"] = True
    st, d = http("POST", "/wiki/search", body, tok)
    assert st == 200, f"search failed on {case['q']}"
    pages = d.get("pages", []) if isinstance(d, dict) else d
    slugs = [p.get("slug") for p in pages]
    gold = case["gold"]
    rank_of_first_gold = next((i + 1 for i, s in enumerate(slugs) if s in gold), None)
    latencies.append(d.get("took_ms") if isinstance(d, dict) else None)
    return {
        "q": case["q"],
        "type": case.get("type"),
        "gold": gold,
        "hit5": rank_of_first_gold is not None and rank_of_first_gold <= K5,
        "hit10": rank_of_first_gold is not None and rank_of_first_gold <= K10,
        "mrr5": (1.0 / rank_of_first_gold) if rank_of_first_gold and rank_of_first_gold <= K5 else 0.0,
        "first_gold_rank": rank_of_first_gold,
        "top5": slugs[:K5],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", help="报告 JSON 另存路径")
    ap.add_argument("--label", default="", help="报告标签（如 v2，对比用）")
    ap.add_argument("--rerank", action="store_true", help="批次④：开启 LLM rerank 精排跑分（开/关对比）")
    args = ap.parse_args()

    bench = json.load(open(os.path.join(HERE, "benchmark.json")))
    tok = login()
    latencies = []
    results = [score_case(tok, c, latencies, rerank=args.rerank) for c in bench["queries"]]

    n = len(results)
    hit5 = sum(r["hit5"] for r in results)
    hit10 = sum(r["hit10"] for r in results)
    mrr5 = sum(r["mrr5"] for r in results) / n
    lat = [l for l in latencies if isinstance(l, (int, float))]

    by_type = {}
    for r in results:
        t = r["type"] or "?"
        by_type.setdefault(t, []).append(r)

    tag = f"{args.label}{' +rerank' if args.rerank else ''}"
    print(f"=== wiki 检索基准报告 {tag} ({time.strftime('%Y-%m-%d %H:%M:%S')}) ===")
    print(f"查询数: {n} | Hit@5: {hit5}/{n} ({hit5/n:.1%}) | Hit@10: {hit10}/{n} ({hit10/n:.1%}) | MRR@5: {mrr5:.3f}")
    if lat:
        print(f"延迟: avg {sum(lat)/len(lat):.0f}ms max {max(lat)}ms")
    for t, rs in sorted(by_type.items()):
        h5 = sum(r["hit5"] for r in rs)
        m = sum(r["mrr5"] for r in rs) / len(rs)
        print(f"  [{t}] Hit@5 {h5}/{len(rs)} MRR@5 {m:.3f}")
    misses = [r for r in results if not r["hit5"]]
    if misses:
        print("--- 未进 top5 的查询 ---")
        for r in misses:
            print(f"  ✗ {r['q']} (gold={r['gold'][:2]}) first@{r['first_gold_rank']} top5={r['top5']}")
    report = {
        "label": args.label, "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "n": n, "hit5": hit5, "hit10": hit10, "mrr5": round(mrr5, 4),
        "latency_avg_ms": round(sum(lat) / len(lat)) if lat else None,
        "by_type": {t: {"hit5": sum(r["hit5"] for r in rs), "n": len(rs), "mrr5": round(sum(r["mrr5"] for r in rs) / len(rs), 4)} for t, rs in by_type.items()},
        "misses": [{"q": r["q"], "first_gold_rank": r["first_gold_rank"], "top5": r["top5"]} for r in misses],
        "results": results,
    }
    if args.out:
        json.dump(report, open(args.out, "w"), ensure_ascii=False, indent=1)
        print(f"报告已存: {args.out}")


if __name__ == "__main__":
    main()
