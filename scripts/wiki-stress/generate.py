#!/usr/bin/env python3
"""wiki 规模化压测数据生成器——合成页面+双链+原料，直接产出 SQL。

用法:
    python3 generate.py --pages 1000 --out stress-1k.sql
    python3 generate.py --pages 10000 --out stress-10k.sql

设计:
- 类型分布: concept 40% / entity 30% / source 15% / synthesis 10% / comparison 5%
- folder: topic-{00..19}/{concept|entity|source|synthesis|comparison} 共 100 个（模拟社区结构）
- 双链: 每页 3-8 条 wikilink，80% 指向同 topic（社区内聚）、20% 全局随机（桥接边）
- tsv: 导入时直接 to_tsvector('simple', slug||' '||title||' '||content)——
  与 service::page_tsv_text 的 EN-63 口径一致（simple 配置 + 三段拼接）
- library_id: SQL 子查询动态取 (SELECT id FROM wiki_libraries WHERE slug='main')
- wiki_sources: 每页 1 条 status='ready' 的原料行（sha256 唯一）
"""
import argparse
import hashlib
import random
import uuid

TOPICS = 20
CATS = ["concept", "entity", "source", "synthesis", "comparison"]
TYPE_WEIGHTS = [("concept", 40), ("entity", 30), ("source", 15), ("synthesis", 10), ("comparison", 5)]

CN_WORDS = [
    "透传原则", "检测链路", "流量识别", "语义指纹", "置信度校准", "社区发现", "双链图谱",
    "增量织入", "原料分块", "向量检索", "全文检索", "重排序", "路由分发", "工单分流",
    "故障定级", "巡检报告", "指纹采集", "对抗样本", "越狱防护", "上下文压缩",
]
EN_WORDS = [
    "pipeline", "router", "calibration", "embedding", "rerank", "classifier", "guardrail",
    "synthesis", "namespace", "snapshot", "fanout", "eval", "baseline", "throughput",
]
SENTENCE_TEMPLATES = [
    "本页记录{cn}的核心机制，相关实现参考 {en} 模块。",
    "{cn}与{cn2}在实践中常常成对出现，{en} 层负责边界校验。",
    "当 {en} 触发阈值时，{cn}流程会降级到人工复核通道。",
    "设计要点：{cn}的置信度分布应保持校准，避免 {en} 端过度自信。",
    "排查路径：先检查 {en} 的输入约束，再回溯{cn}的语义指纹。",
    "与相邻概念的差异：{cn}强调结构保证，{cn2}强调统计一致性。",
]


def pick_type(rng: random.Random) -> str:
    total = sum(w for _, w in TYPE_WEIGHTS)
    roll = rng.randint(1, total)
    acc = 0
    for t, w in TYPE_WEIGHTS:
        acc += w
        if roll <= acc:
            return t
    return "concept"


def gen_content(rng: random.Random, title: str, links: list[str]) -> str:
    paras = []
    for _ in range(rng.randint(3, 6)):
        sents = []
        for _ in range(rng.randint(2, 4)):
            tpl = rng.choice(SENTENCE_TEMPLATES)
            s = tpl.format(
                cn=rng.choice(CN_WORDS),
                cn2=rng.choice(CN_WORDS),
                en=rng.choice(EN_WORDS),
            )
            sents.append(s)
        paras.append("".join(sents))
    body = "\n\n".join(paras)
    link_lines = "\n".join(f"- 相关：[[{slug}]]" for slug in links)
    return f"{body}\n\n## 关联页面\n\n{link_lines}\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pages", type=int, default=1000)
    ap.add_argument("--out", default="stress.sql")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()
    rng = random.Random(args.seed)

    pages = []  # (slug, title, page_type, folder, content)
    for i in range(args.pages):
        ptype = pick_type(rng)
        topic = i % TOPICS
        slug = f"stress-{ptype}-{i:05d}"
        title = f"压测{ptype}样本 {i:05d}：{rng.choice(CN_WORDS)}"
        folder = f"topic-{topic:02d}/{ptype}"
        pages.append((slug, title, ptype, folder, i, topic))

    # 双链：先定骨架再生成正文（wikilink 要写进 content）
    by_topic: dict[int, list[int]] = {}
    for slug, _, _, _, idx, topic in pages:
        by_topic.setdefault(topic, []).append(idx)
    all_idx = [p[4] for p in pages]
    slug_of = {p[4]: p[0] for p in pages}

    rows = []
    src_rows = []
    for slug, title, ptype, folder, idx, topic in pages:
        n_links = rng.randint(3, 8)
        same = [j for j in by_topic[topic] if j != idx]
        links = set()
        for _ in range(n_links):
            pool = same if rng.random() < 0.8 else all_idx
            j = rng.choice(pool)
            if j != idx:
                links.add(j)
        link_slugs = sorted(slug_of[j] for j in links)
        content = gen_content(rng, title, link_slugs)
        content_sql = content.replace("'", "''")
        title_sql = title.replace("'", "''")
        pid = str(uuid.uuid4())
        rows.append(
            f"('{pid}','{slug}','{title_sql}','{ptype}','{content_sql}',"
            f"'{{\"sources\": [], \"title\": \"{title_sql}\", \"via\": \"ai\"}}'::jsonb,"
            f"'human','{folder}',"
            f"(SELECT id FROM wiki_libraries WHERE slug='main'),"
            f"to_tsvector('simple', '{slug} {title_sql} {content_sql}'.replace_ext))"
        )
        # 修正：上面 replace_ext 是占位错误——改为直接三段拼接
        rows[-1] = (
            f"('{pid}','{slug}','{title_sql}','{ptype}','{content_sql}',"
            f"'{{\"sources\": [], \"title\": \"{title_sql}\", \"via\": \"ai\"}}'::jsonb,"
            f"'human','{folder}',"
            f"(SELECT id FROM wiki_libraries WHERE slug='main'),"
            f"to_tsvector('simple', '{slug} {title_sql} ' || '{content_sql}'))"
        )
        sha = hashlib.sha256(f"stress-src-{idx}".encode()).hexdigest()
        src_rows.append(
            f"('{uuid.uuid4()}','{sha}','/tmp/wiki-stress/{slug}.md','{title_sql} 来源','ready',"
            f"now(),NULL,(SELECT id FROM wiki_libraries WHERE slug='main'))"
        )

    with open(args.out, "w", encoding="utf-8") as f:
        f.write("-- wiki 规模化压测数据（generate.py 生成）\nBEGIN;\n")
        for i in range(0, len(rows), 200):
            f.write("INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, folder, library_id, tsv) VALUES\n")
            f.write(",\n".join(rows[i : i + 200]))
            f.write(";\n")
        for i in range(0, len(src_rows), 200):
            f.write("INSERT INTO wiki_sources (id, sha256, raw_path, title, status, last_ingested_at, error, library_id) VALUES\n")
            f.write(",\n".join(src_rows[i : i + 200]))
            f.write(";\n")
        f.write("COMMIT;\n")
    print(f"生成 {len(rows)} 页 + {len(src_rows)} 原料 → {args.out}")


if __name__ == "__main__":
    main()
