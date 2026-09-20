#!/usr/bin/env python3
"""按域拆分单文件模块（架构治理 task-4）：impl 方法重分布 + 顶层项搬迁。

用法：
    python3 scripts/quality/split_engine_modules.py --dry     # 看归类与体积
    python3 scripts/quality/split_engine_modules.py --apply

机制：
- 目标文件解析为顶层项；`impl Type {` 项按方法切开逐方法归类，其余项整项归类；
- 归到子模块的东西写进 `<父文件名>/<子模块>.rs`（Rust 子模块目录约定），
  顶部 `use super::*;` 继承父模块（含其 use 与私有项）；
- 父文件保留未搬迁项 + `mod <子>;` + `pub use <子>::*;`（保持 crate::父::名字 路径可用）；
- impl 内被搬走的方法补 `pub(super)`（保持原 `pub` 不变），跨子模块互调可见。
"""
import argparse
import json
import re
from pathlib import Path

TOP = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|struct|enum|trait|impl|const|static|type|use|mod|macro_rules!)\b"
)
METHOD = re.compile(r"^    (?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)")
NAME = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|struct|enum|trait|const|static|type|mod)\s+([A-Za-z0-9_]+)"
)

# ---------------- 配置：文件 → {impl 类型, 子模块 → 成员名单} ----------------
CONFIGS = {
    "crates/wiki-engine/src/service.rs": {
        "impl": "WikiService",
        "modules": {
            "pages": [
                "put_page", "restore_page_version", "list_pages", "merge_pages", "delete_page",
                "page_version_content", "page_versions", "get_page", "resolve_slug", "list_folders",
            ],
            "ingest_ops": [
                "ingest", "ingest_document", "auto_ingest_page", "index", "rebuild_all_links",
                "backfill_tsv", "duplicate_candidates", "archive_answer", "archive_query",
                "reingest", "backfill_embeddings",
            ],
            "search": [
                "search_opts", "graph_filtered", "rerank_with_graph", "load_adjacency",
                "search_with_purpose", "query_gaps", "search", "search_reranked", "graph",
            ],
            "repair_ops": [
                "repair", "delete_source_cascade", "review_resolve", "audit", "reviews",
                "list_sources", "lint", "lint_deep_enqueue", "prune",
            ],
        },
    },
    "crates/core/src/memory.rs": {
        "impl": "MemoryService",
        "modules": {
            "sessions": [
                "write_session_identity", "unvoid_session", "void_session", "append_session",
                "import_session", "erase_session", "unvoid_sessions", "erase_sessions",
                "list_sessions", "list_sessions_meta", "get_session", "write_session",
                "arm_deep_purge",
            ],
            "atoms": [
                "update_atom", "create_atom", "parse_import", "correct_atom", "distill_result",
                "discard_review", "list_atoms", "confirm_review", "atom_revisions",
            ],
            "search": [
                "context_pack", "search", "try_embed", "try_embed_query", "fire_hit_feedback",
                "reembed", "embedding_status",
            ],
            "entity": [
                "update_entity", "persona_rollback", "create_entity", "persona_edit",
                "create_relation", "persona_repin", "get_entity", "delete_entity",
                "persona_unpin", "list_relations", "forget_entity", "entity_row",
                "list_entities", "entity_graph", "merge_entities", "delete_relation",
                "entity_revisions", "attach_atom", "detach_atom",
            ],
            "ops": [
                "trigger_distill", "trigger_distill_manual", "rhythm_status", "kv_put", "kv_get",
                "kv_list", "kv_search", "export", "audit", "timeline", "purge_agent",
                "purge_deep", "list_scenarios", "get_scenario", "persona", "persona_history",
            ],
        },
    },
    "crates/wiki-engine/src/ingest.rs": {
        "impl": None,
        "modules": {
            "generate": [
                "generate_job", "mark_source_failed", "source_failure_msg",
                "MAX_PAGES_PER_GENERATE", "GEN_SLICE_CHARS",
            ],
            "pages": [
                "update_index_and_log", "rebuild_links", "upsert_system_page",
                "rebuild_overview_page", "rebuild_index_page", "read_index", "title_of",
                "read_source",
            ],
        },
    },
    "crates/core/src/wiki_docs/pipeline.rs": {
        "impl": None,
        "modules": {
            "util": [
                "normalize_url", "mime_from_name", "extract_title_from_html", "hex", "data_uploads",
            ],
        },
    },
}


def strip_strings(line):
    return re.sub(r'"(?:[^"\\]|\\.)*"', "", line)


def code_lines(text):
    n, inb = 0, False
    for l in text.split("\n"):
        s = l.strip()
        if not s:
            continue
        if inb:
            if "*/" in s:
                inb = False
            continue
        if s.startswith("/*"):
            if "*/" not in s:
                inb = True
            continue
        if s.startswith("//"):
            continue
        n += 1
    return n


def item_extent(lines, i, n):
    kw = None
    for k in ("use", "mod", "const", "static", "type", "fn", "struct", "enum", "trait", "impl"):
        if re.match(rf"^(?:pub\S*\s+)?(?:async\s+)?{k}\b", lines[i]):
            kw = k
            break
    if kw in ("use", "mod", "const", "static", "type"):
        k = i
        while k < n and not lines[k].rstrip().endswith(";"):
            k += 1
        return k
    depth, started, k = 0, False, i
    while k < n:
        c = strip_strings(lines[k])
        depth += c.count("{") - c.count("}")
        if "{" in c:
            started = True
        if started and depth <= 0:
            return k
        k += 1
    return n - 1


def split_top(lines):
    items, pending, i, n = [], [], 0, len(lines)
    while i < n:
        raw = lines[i]
        st = raw.strip()
        if not st or st.startswith("#[") or st.startswith("//"):
            pending.append(raw)
            i += 1
            continue
        if TOP.match(raw.rstrip()):
            k = item_extent(lines, i, n)
            items.append({"text": pending + lines[i : k + 1], "head": raw, "kind": kind_of(raw), "name": name_of(raw)})
            pending = []
            i = k + 1
            continue
        pending.append(raw)
        i += 1
    if pending:
        if items:
            items[-1]["text"].extend(pending)
        else:
            items.append({"text": pending, "head": "", "kind": "?", "name": "?"})
    return items


def kind_of(l):
    for kw in ("fn", "struct", "enum", "trait", "impl", "const", "static", "type", "use", "mod"):
        if re.match(rf"^(?:pub\S*\s+)?(?:async\s+)?{kw}\b", l):
            return kw
    return "?"


def name_of(l):
    m = NAME.match(l)
    if m:
        return m.group(1)
    m = re.match(r"^(?:pub\s+)?(?:unsafe\s+)?impl\b\s*(.*)$", l)
    return ("impl:" + m.group(1)[:40]) if m else "?"


def split_methods(text):
    """把 impl 块体切成方法（含前置属性/文档注释）。"""
    methods, pending = [], []
    i, n = 1, len(text) - 1
    while i < len(text):
        if re.match(r"^(?:pub\s+)?(?:unsafe\s+)?impl\b", text[i]):
            i += 1
            break
        i += 1
    while i < n:
        raw = text[i]
        st = raw.strip()
        if not st or st.startswith("#[") or st.startswith("//") or st == "}":
            pending.append(raw)
            i += 1
            continue
        if METHOD.match(raw):
            depth, started, k = 0, False, i
            while k < n:
                c = strip_strings(text[k])
                depth += c.count("{") - c.count("}")
                if "{" in c:
                    started = True
                if started and depth <= 0:
                    break
                k += 1
            methods.append({"text": pending + text[i : k + 1], "name": METHOD.match(raw).group(1)})
            pending = []
            i = k + 1
            continue
        pending.append(raw)
        i += 1
    if pending and methods:
        methods[-1]["text"].extend(pending)
    return methods



DECL = re.compile(r"^(?:async\s+)?(?:unsafe\s+)?(?:fn|const|static|struct|enum|type)\b")


def visible(text: str) -> str:
    """把搬走的顶层项的声明行提升为 pub(super)（原本 pub 的不动）。"""
    lines = text.split("\n")
    for idx, l in enumerate(lines):
        if l.strip().startswith(("//", "#[")):
            continue
        if DECL.match(l.strip()) and not l.strip().startswith("pub"):
            indent = l[: len(l) - len(l.lstrip())]
            lines[idx] = indent + "pub(super) " + l.strip()
        break
    return "\n".join(lines)


def dedent(text, n=4):
    return "\n".join(l[n:] if l.startswith(" " * n) else l for l in text.split("\n"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry", action="store_true")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    for file, cfg in CONFIGS.items():
        src = Path(file)
        lines = src.read_text().split("\n")
        items = split_top(lines)
        impl_name = cfg.get("impl")
        buckets: dict[str, list[str]] = {m: [] for m in cfg["modules"]}
        member2mod = {n: m for m, names in cfg["modules"].items() for n in names}
        kept: list[str] = []
        moved_methods: dict[str, list[dict]] = {m: [] for m in cfg["modules"]}

        for it in items:
            if impl_name and it["kind"] == "impl" and re.match(rf"^(?:pub\s+)?(?:unsafe\s+)?impl\s+{impl_name}\b", it["head"].strip()):
                methods = split_methods(it["text"])
                # impl 头部（含属性）与尾部 } 留在父文件
                head_len = next((idx for idx, l in enumerate(it["text"]) if re.match(r"^(?:pub\s+)?(?:unsafe\s+)?impl\b", l)), 0)
                kept.append("\n".join(it["text"][:head_len]).rstrip())
                kept.append(it["text"][head_len])  # `impl Type {`（保留未搬走方法的归属块）
                for m in methods:
                    dest = member2mod.get(m["name"])
                    if dest:
                        moved_methods[dest].append(m)
                    else:
                        kept.append("\n".join(m["text"]))
                kept.append("}")
                continue
            if it["kind"] in ("use", "mod"):
                kept.append("\n".join(it["text"]))
                continue
            dest = member2mod.get(it["name"])
            if dest:
                buckets[dest].append("\n".join(it["text"]))
            else:
                kept.append("\n".join(it["text"]))

        print(f"\n===== {file} =====")
        for m in cfg["modules"]:
            body = "\n".join(buckets[m]) + "\n".join(
                dedent("\n".join(x["text"])) for x in moved_methods[m]
            )
            print(f"  [子模块 {m}] {len(buckets[m])} 顶层项 / {len(moved_methods[m])} 方法 / {code_lines(body)} 代码行")
        print(f"  [父文件保留] {code_lines(chr(10).join(kept))} 代码行")

        if not args.apply:
            continue

        # ---- 写子模块 ----
        parent_stem = src.stem
        outdir = src.parent / parent_stem
        outdir.mkdir(exist_ok=True)
        for m in cfg["modules"]:
            parts = [
                f"//! `{parent_stem}` 的实现切片（架构治理 2026-09-20：自 {parent_stem}.rs 纯搬移，零行为变化）。\n",
                "use super::*;\n",
            ]
            if impl_name and moved_methods[m]:
                parts.append(f"impl {impl_name} {{")
                parts.append(
                    "\n\n".join(
                        re.sub(r"(?m)^(    )(?:pub(?:\([^)]*\))?\s+)?((?:async\s+)?fn )", r"\1pub(super) \2", dedent("\n".join(x["text"])))
                        for x in moved_methods[m]
                    )
                )
                parts.append("}")
            if buckets[m]:
                parts.append("\n".join(visible(x) for x in buckets[m]))
            (outdir / f"{m}.rs").write_text("\n".join(parts) + "\n")
            print(f"  写入 {outdir / (m + '.rs')}")

        # ---- 重写父文件 ----
        decls = [f"mod {m};" for m in cfg["modules"]]
        reexports = [f"pub use {m}::*;" for m in cfg["modules"] if buckets[m]]
        # crate 级 //! 文档必须留在文件最顶（内层文档注释只能出现在开头）
        inner_docs: list[str] = []
        for l in kept:
            if l.strip().startswith("//!"):
                inner_docs.append(l)
            else:
                break
        kept = [l for l in kept if l not in inner_docs]
        header = [l for l in kept if l.strip().startswith(("use ", "pub use "))]
        rest = [l for l in kept if l not in header]
        out = inner_docs + [""] + decls + reexports + [""] + header + [""] + rest
        src.write_text("\n".join(out) + "\n")
        print(f"  重写 {src}")

    if not args.apply:
        print("\n（dry-run）")


if __name__ == "__main__":
    main()
