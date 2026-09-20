#!/usr/bin/env python3
"""确定性按域拆分单文件模块（架构治理 wave-2，2026-09-21）。

与 `split_engine_modules.py` 的差异——修掉上一波暴露的 4 个确定性缺陷：

1. **属性/空行感知**：项起始包含前导 `#[...]` / `///` 块，可见性提升落在**声明行**
   （不会被声明前的空行/注释打断，旧脚本在此静默失效）。
2. **方法搬迁可见性**：方法先 dedent 再补 `pub(super)`（旧脚本先 dedent 后匹配，
   正则永不命中 → 方法保持私有 → E0624）。
3. **保护不重排**：`mod tests` 与既有 `mod <子>;`/`pub use` 一律留在父文件原文，
   仅通过**行区间删除**搬迁目标项，父文件其余内容逐字保持（旧脚本重排 kept 列表，
   会把 in-file 测试模块内部的 `use` 行抽走并截断模块）。
4. **幂等守卫**：父文件已含本次目标 `mod <子>;` 则整文件跳过（旧脚本重复运行会
   二次重写声明 → E0428）。

用法：python3 scripts/quality/split_domain_modules.py --dry | --apply
"""

import argparse
import re
from pathlib import Path

ATTR = re.compile(r"^\s*(#\[|///|//!)")
DECL_RE = re.compile(
    r"^(?P<vis>pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const)\s+)*"
    r"(?P<kw>fn|struct|enum|trait|impl|const|static|type|union|mod)\b"
)
USE_RE = re.compile(r"^\s*(?:pub\s+)?use\b")
NAME_RE = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const)\s+)*"
    r"(?:fn|struct|enum|trait|const|static|type|union|mod)\s+(?P<n>[A-Za-z0-9_]+)"
)
IMPL_RE = re.compile(r"^(?:pub\s+)?(?:unsafe\s+)?impl(?:<[^>]*>)?\s+(?:[A-Za-z0-9_:<>,\s]+\s+for\s+)?(?P<t>[A-Za-z0-9_]+)")
STR_RE = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
LINE_COMMENT = re.compile(r"//.*$")

# ---------------- 配置：文件 → {impl 类型, 子模块 → 成员名单} ----------------
CONFIGS = {
    "crates/storage/src/repo/memory.rs": {
        "impl": None,
        "modules": {
            "sessions": [
                "insert_session", "insert_session_identity", "insert_session_import", "find_session",
                "list_sessions", "append_session_update", "delete_session", "session_distill_status",
                "void_session_update", "unvoid_session_update", "session_backlog",
                "rhythm_last_heartbeat", "reset_processing_sessions", "list_all_sessions",
                "session_distill_meta", "list_sessions_meta",
            ],
            "atoms": [
                "find_atom", "list_atoms", "find_active_atom", "insert_atom", "correct_atom",
                "review_confirm", "review_discard", "count_running_extract", "count_running_persona",
                "atoms_by_session", "AtomLiteralHit", "atoms_literal_fallback", "atom_revisions",
                "insert_atom_revision", "update_atom_full", "count_atom", "archive_atoms_by_entity",
                "archive_atoms_by_session", "restore_atoms_by_session", "list_atoms_all",
                "atoms_by_ids", "recent_active_atoms", "pending_review_atoms", "list_atom_refs_like",
                "update_atom_source_refs", "bump_hit_counts",
            ],
            "scenarios": ["list_scenarios", "find_scenario", "list_scenarios_all"],
            "persona": [
                "persona_current", "persona_history", "persona_all", "persona_max_version",
                "persona_latest", "insert_persona_pinned", "insert_persona_rollback",
                "persona_unpin", "persona_version",
            ],
            "entity": [
                "entity_row", "list_entities", "entity_atoms", "entity_scenarios", "entity_neighbors",
                "entity_relations", "list_relations", "entity_id_by_name_kind", "insert_entity",
                "entity_summary", "update_entity_name", "insert_entity_revision", "update_entity_summary",
                "pin_entity", "delete_entity", "live_entity_id", "detach_entity_links",
                "attach_atom_link", "touch_entity", "merge_entities_tx", "cooccurrence_edges",
                "entities_by_ids", "entities_for_export", "insert_relation", "delete_relation",
                "entity_revisions", "detach_atom_link", "archive_orphan_entities", "revive_entity",
            ],
            "kv": ["kv_upsert", "kv_get", "kv_list", "kv_search_literal"],
            "ops": ["purge_agent_tx", "purge_deep", "timeline", "embedding_missing_counts", "audit"],
        },
    },
    "crates/storage/src/repo/transfer.rs": {
        "impl": None,
        "modules": {
            "export": [
                "export_wiki_pages", "export_wiki_libraries", "export_projects", "export_todos",
                "export_skills_with_files", "export_wiki_promotions",
            ],
            "import_memory": [
                "import_session", "import_atom", "backfill_atom_superseded_by", "import_scenario",
                "import_persona", "import_entity", "import_entity_relation", "import_kv_entries",
            ],
            "import_wiki": [
                "import_wiki_library", "main_library_id", "import_wiki_page", "import_wiki_promotions",
            ],
            "import_ops": [
                "import_todos", "import_skill", "import_project", "import_project_location",
                "import_project_doc", "TodoExportRow",
            ],
            "util": ["ts", "id_of", "str_of"],
        },
    },
    "crates/core/src/project.rs": {
        "impl": "ProjectService",
        "modules": {
            "projects": [
                "new", "default_categories", "list_types", "create_project", "list_projects",
                "get_project", "project_id_by_name", "update_project", "delete_project",
                "batch_delete_projects", "get_project_bare", "add_location", "update_location",
                "delete_location", "get_location", "category_error",
            ],
            "docs": [
                "validate_doc_folder", "add_doc", "update_doc", "delete_doc", "get_doc",
                "read_doc_lines", "search_doc_lines", "patch_doc",
            ],
            "files": [
                "validate_file_name", "infer_mime", "upsert_file", "list_files", "get_file",
                "delete_file", "list_file_versions", "get_file_version",
            ],
            "model": [
                "PROJECT_TYPES", "ProjectTypeDto", "light_normalize", "is_whole_word_hit",
                "normalize_for_search", "DocLineHitDto", "ProjectDetailDto",
            ],
        },
    },
    "crates/core/src/skills.rs": {
        "impl": "SkillsService",
        "modules": {
            "crud": [
                "new", "create_skill", "list_skills", "get_skill", "get_skill_with_content",
                "update_skill", "delete_skill", "resolve", "resolve_slug", "validate_name",
                "validate_two_kind", "reject_script_ops",
            ],
            "files": ["list_files", "get_file", "put_file", "delete_file", "validate_file_path"],
            "revisions": ["list_revisions", "restore_revision"],
            "import_export": ["import_skills", "import_one", "export_one", "export_skills"],
            "model": [
                "LOCAL_PATH_MAX", "parse_tags_value", "assign_meta", "parse_frontmatter", "slugify",
                "valid_slug", "SkillImportItem", "SkillImportReport", "SKILL_FILE_MAX_CHARS",
                "SkillFileEntryDto", "SkillExportDto", "render_skill_md", "NewSkill", "SkillPatch",
                "NormalizedKind",
            ],
        },
    },
    "crates/llm/src/provider.rs": {
        "impl": "OpenAiCompatProvider",
        "modules": {
            "chat": ["new", "post_with_retry", "chat", "embed", "name"],
            "wire": [
                "LlmProvider", "CircuitState", "ChatApiResp", "ChatChoice", "ApiUsage", "EmbedApiResp",
                "EmbedItem", "retry_after_secs", "normalize_base_url", "classify_http_error",
                "fetch_model_ids",
            ],
        },
    },
    "crates/cg-bridge/src/bridge.rs": {
        "impl": "CgBridge",
        "modules": {
            "version": [
                "new", "detect_version", "ensure_version", "ensure_ready", "set_status",
                "mark_all_version_mismatch", "cli_status", "read_stats",
            ],
            "index": ["register", "upload_artifact", "index", "sync", "gc", "register_handlers"],
            "query": [
                "freshness_for", "list", "get", "query", "explore_outline", "full_graph", "graph",
                "delete",
            ],
            "model": [
                "CgProjectDto", "CliStatus", "CgSymbolRef", "CallersShape", "CalleesShape",
                "CliOutput", "index_db_path", "index_usable", "normalize_callgraph", "truncate",
                "classify",
            ],
        },
    },
    "crates/api/src/routes/memory_api.rs": {
        "impl": None,
        "modules": {
            "sessions": [
                "WriteSessionRequest", "default_distill", "write_session", "ImportSessionRequest",
                "import_session", "ListSessionsParams", "list_sessions", "get_session", "erase_session",
                "void_session", "restore_session", "BatchIdsRequest", "batch_restore_sessions",
                "batch_erase_sessions", "validate_batch", "AppendSessionRequest", "append_session",
            ],
            "atoms": [
                "ListAtomsParams", "list_atoms", "CreateAtomRequest", "create_atom",
                "UpdateAtomRequest", "update_atom", "atom_revisions", "DistillRequest",
                "trigger_distill", "HeartbeatParams", "rhythm_heartbeat", "rhythm_status",
                "embedding_status", "reembed_memory", "SearchRequest", "search", "ContextParams",
                "context",
            ],
            "entities": [
                "CreateRelationRequest", "list_entity_relations", "create_entity_relation",
                "delete_entity_relation", "BatchEntitiesRequest", "batch_entities", "export_entities",
                "ListEntitiesParams", "SearchEntitiesParams", "search_entities_handler",
                "list_entities", "entity_graph", "CreateEntityRequest", "create_entity", "get_entity",
                "UpdateEntityRequest", "update_entity", "ForgetParams", "delete_entity",
                "attach_atom", "detach_atom", "MergeEntityRequest", "merge_entity", "entity_revisions",
            ],
            "persona": [
                "list_scenarios", "get_scenario", "get_persona", "HistoryParams", "persona_history",
                "RollbackRequest", "persona_rollback", "PersonaEditRequest", "persona_edit",
            ],
            "kv": ["ListKvParams", "list_kv", "get_kv"],
            "ops": [
                "require_cron", "require_erase", "me", "svc", "default_conf", "parse_flex_datetime",
                "opt_flex_dt", "PurgeRequest", "purge_agent", "ExportParams", "export_memory",
                "TimelineParams", "timeline", "actor_of",
            ],
        },
    },
    "crates/api/src/routes/llm_api.rs": {
        "impl": None,
        "modules": {
            "providers": [
                "require_llm", "cipher_from", "CreateProviderRequest", "default_capability",
                "ProviderDto", "create_provider", "UpdateProviderRequest", "update_provider",
                "delete_provider", "ReEncryptRequest", "ReEncryptResult", "reencrypt_providers",
                "list_providers", "TestResult", "test_provider",
            ],
            "routing": [
                "get_routing", "RoutingSuggestRequest", "suggest_routing", "parse_llm_json",
                "put_routing",
            ],
            "keys": [
                "CreateApiKeyRequest", "ApiKeyCreated", "create_api_key_handler", "ApiKeyDto",
                "list_api_keys", "revoke_api_key", "BatchRevokeRequest", "BatchRevokeResult",
                "UpdateApiKeyRequest", "key_dto", "update_api_key", "batch_revoke_api_keys",
            ],
            "usage": ["usage", "FetchModelsRequest", "fetch_models"],
        },
    },
    "crates/api/src/routes/wiki_api.rs": {
        "impl": None,
        "modules": {
            "pages": [
                "we", "pe", "main_lib", "ListPagesParams", "list_pages", "list_folders", "get_page",
                "PutPageRequest", "put_page", "delete_page", "rebuild_links", "rebuild_tsv",
                "MergePagesRequest", "merge_pages", "duplicates",
            ],
            "ingest": [
                "IngestRequest", "ingest", "IngestAccepted", "WikiSourceDto", "list_sources",
                "delete_source", "PromoteRequest", "promote", "PromotionsParams", "promotions",
                "ArchiveQueryRequest", "archive_query",
            ],
            "search_graph": [
                "GraphFilterParams", "graph", "WikiSearchRequest", "search", "query_gaps",
                "SetPurposeRequest", "get_purpose", "set_purpose",
            ],
            "repair_review": [
                "lint", "repair", "repair_async", "ApplyProposalRequest", "apply_proposal",
                "list_proposals", "list_reviews", "ResolveReviewRequest", "resolve_review",
                "insights", "DismissInsightRequest", "dismiss_insight", "reset_insights",
            ],
            "ops": ["svc"],
        },
    },
    "crates/distill/src/persona.rs": {
        "impl": None,
        "modules": {
            "inputs": [
                "Resolved", "resolve_inputs", "payload_bool", "payload_uuids", "payload_strings",
                "load_scenarios", "load_current_aspects", "load_pinned",
            ],
            "prompt": [
                "build_persona_prompt", "persona_with_llm", "parse_aspects", "store_aspects",
                "write_empty_versions", "retire_by_removal",
            ],
            "evidence": [
                "load_evidence_maps", "uuid_array", "session_ids_of", "resolve_evidence_ids",
                "build_evidence", "AspectEntry",
            ],
        },
    },
}


def clean_lines(lines):
    """有状态清洗：把字符串字面量/块注释/行注释内的字符去掉（跨行状态保持）。

    逐行剥离字符串在**跨行字符串**上会失效（eg `format!("...{\"k\":...")` 跨行），
    导致花括号计数漂移、项边界溢出——这里一次性按状态机扫描整份文件。
    """
    out = []
    state = None          # None | "str" | "block" | int(raw 的 # 个数)
    for line in lines:
        res, i, n = [], 0, len(line)
        while i < n:
            if state == "block":
                j = line.find("*/", i)
                if j < 0:
                    i = n
                    break
                state = None
                i = j + 2
                continue
            if state == "str":
                c = line[i]
                if c == "\\":
                    i += 2
                    continue
                if c == '"':
                    state = None
                i += 1
                continue
            if isinstance(state, int):
                if line.startswith('"' + "#" * state, i):
                    i += 1 + state
                    state = None
                    continue
                i += 1
                continue
            # 正常代码区
            if line.startswith("//", i):
                break
            if line.startswith("/*", i):
                state = "block"
                i += 2
                continue
            m = re.match(r'(?:b|br|r)?(#*)"', line[i:])
            if m and m.group(0)[0] in "br":
                state = len(m.group(1))
                i += m.end()
                continue
            c = line[i]
            if c == '"':
                state = "str"
                i += 1
                continue
            if c == "'":                      # 字符字面量（生命周期不当作字符串）
                if i + 2 < n and line[i + 2] == "'":
                    i += 3
                    continue
                if i + 3 < n and line[i + 1] == "\\" and line[i + 3] == "'":
                    i += 4
                    continue
            res.append(c)
            i += 1
        out.append("".join(res))
    return out


def delta(clean_line: str) -> int:
    return clean_line.count("{") - clean_line.count("}")


def parse_items(lines, clean=None):
    """顶层项：[{start, end, kind, name, decl}]，属性/文档行并入其下声明所属的项。"""
    clean = clean if clean is not None else clean_lines(lines)
    items, i, n = [], 0, len(lines)
    marks = mark_attr_lines(clean, lines)
    while i < n:
        if marks[i] or not lines[i].strip():
            i += 1
            continue
        decl = lines[i]
        m = DECL_RE.match(decl)
        if not m:
            i += 1
            continue
        kw = m.group("kw")
        name_m = NAME_RE.match(decl)
        name = name_m.group("n") if name_m else None
        k, depth, started = i, 0, False
        while k < n:
            depth += delta(clean[k])
            if "{" in clean[k]:
                started = True
            if started and depth <= 0:
                break
            if not started and lines[k].rstrip().endswith(";"):
                break
            k += 1
        start = i
        while start - 1 >= 0 and marks[start - 1]:
            start -= 1
        kind = "use" if USE_RE.match(decl) else ("mod" if kw == "mod" else kw)
        items.append({"start": start, "end": k, "kind": kind, "name": name, "decl": decl})
        i = k + 1
    return items


def mark_attr_lines(clean, lines):
    """属性/文档行标记：`///`/`//!`/`#[` 用**原行**判定，方括号配平用清洗行。"""
    n = len(clean)
    marks = [False] * n
    i = 0
    while i < n:
        t = lines[i].strip()
        if t.startswith("#["):
            depth = 0
            j = i
            while j < n:
                depth += clean[j].count("[") - clean[j].count("]")
                marks[j] = True
                if depth <= 0:
                    break
                j += 1
            i = j + 1
            continue
        if t.startswith(("///", "//!")):
            marks[i] = True
        i += 1
    return marks


def parse_methods(lines, item):
    """impl 项 → 方法列表 [{start,end,name,decl}]（成员缩进由首个 fn 决定）。"""
    body = lines[item["start"]:item["end"] + 1]
    out, k = [], 0
    member_indent = None
    for idx, l in enumerate(body):
        m = re.match(r"^(\s+)(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe)\s+)*fn\s+(?P<n>[A-Za-z0-9_]+)", l)
        if m:
            member_indent = len(m.group(1))
            break
    if member_indent is None:
        return out
    idx = 0
    while idx < len(body):
        l = body[idx]
        if re.match(rf"^ {{{member_indent}}}(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe)\s+)*fn\s+[A-Za-z0-9_]+", l):
            # 回溯属性/文档块
            s = idx
            while s - 1 >= 0 and ATTR.match(body[s - 1]):
                s -= 1
            k, depth, started = idx, 0, False
            cbody = clean_lines(body)
            while k < len(body):
                depth += delta(cbody[k])
                if "{" in strip_noise(body[k]):
                    started = True
                if started and depth <= 0:
                    break
                k += 1
            nm = re.match(rf"^ {{{member_indent}}}(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe)\s+)*fn\s+([A-Za-z0-9_]+)", l).group(1)
            owner = next((b.strip() for b in body if re.match(r"^(?:pub\s+)?(?:unsafe\s+)?impl\b", b.strip())), "")
            out.append({"start": item["start"] + s, "end": item["start"] + k, "name": nm,
                        "text": body[s:k + 1], "indent": member_indent, "owner": owner})
            idx = k + 1
            continue
        idx += 1
    return out


def promote_decl(text: str, vis: str = "pub(super) ") -> str:
    """把项声明行提升为 vis（已有 pub 则不动）；属性/文档行保持原样。"""
    lines = text.split("\n")
    for i, l in enumerate(lines):
        if ATTR.match(l) or not l.strip():
            continue
        if DECL_RE.match(l.strip()) and not l.strip().startswith("pub"):
            indent = l[: len(l) - len(l.lstrip())]
            lines[i] = indent + vis + l.strip()
        break
    return "\n".join(lines)


def promote_fields(text: str) -> str:
    """struct/enum 体内的私有字段提升为 pub(super)（跨子模块访问用）。"""
    lines = text.split("\n")
    out, depth, body_at = [], 0, None
    for l in lines:
        s = l.strip()
        if body_at is None and re.match(r"^(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\b", s):
            depth += delta(l)
            body_at = depth
            out.append(l)
            continue
        if body_at is not None and depth == body_at and re.match(r"^[A-Za-z_][A-Za-z0-9_]*\s*:", s) \
                and not s.startswith(("pub", "//", "#")):
            indent = l[: len(l) - len(l.lstrip())]
            out.append(indent + "pub(super) " + s)
        else:
            out.append(l)
        depth += delta(l)
        if body_at is not None and depth < body_at:
            body_at = None
    return "\n".join(out)


def dedent(text: str, n: int) -> str:
    return "\n".join(l[n:] if l.startswith(" " * n) else l for l in text.split("\n"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry", action="store_true")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--only", help="只处理该文件（便于逐文件验证）")
    args = ap.parse_args()

    for file, cfg in CONFIGS.items():
        if args.only and args.only not in file:
            continue
        src = Path(file)
        if not src.exists():
            print(f"  ⚠ 不存在：{file}")
            continue
        text = src.read_text()
        lines = text.split("\n")
        mods = list(cfg["modules"])
        # 幂等守卫
        if all(re.search(rf"^mod {m};", text, re.M) for m in mods):
            print(f"  ⏭ {file} 已拆分，跳过")
            continue
        items = parse_items(lines, clean_lines(lines))
        member2mod = {n: m for m, names in cfg["modules"].items() for n in names}
        moves = []          # 待删除行区间
        buckets = {m: [] for m in mods}
        method_buckets = {m: [] for m in mods}
        kept_names = []
        for it in items:
            if it["kind"] in ("use", "mod"):
                continue
            if it["kind"] == "impl" and cfg["impl"]:
                m_impl = IMPL_RE.match(it["decl"].strip())
                if m_impl and m_impl.group("t") == cfg["impl"]:
                    ms = parse_methods(lines, it)
                    moved = [x for x in ms if member2mod.get(x["name"])]
                    for meth in moved:
                        method_buckets[member2mod[meth["name"]]].append(meth)
                    for meth in ms:
                        if not member2mod.get(meth["name"]):
                            kept_names.append(meth["name"])
                    if ms and len(moved) == len(ms):
                        moves.append((it["start"], it["end"]))   # 整块搬空 → 连 impl 头一起删
                    else:
                        moves.extend((x["start"], x["end"]) for x in moved)
                    continue
            dest = member2mod.get(it["name"])
            if dest:
                buckets[dest].append(it)
                moves.append((it["start"], it["end"]))
            else:
                kept_names.append(it["name"])
        print(f"\n===== {file} =====")
        for m in mods:
            n_items = len(buckets[m]); n_meth = len(method_buckets[m])
            print(f"  [子模块 {m}] {n_items} 顶层项 / {n_meth} 方法")
        print(f"  [父文件保留] {len(kept_names)} 项，删 {len(moves)} 个区间")
        if not args.apply:
            continue
        # ---- 写子模块 ----
        outdir = src.parent / src.stem
        outdir.mkdir(exist_ok=True)
        for m in mods:
            parts = [
                f"//! `{src.stem}` 的实现切片（架构治理 2026-09-21：自 {src.stem}.rs 纯搬移，零行为变化）。",
                "",
                "use super::*;",
                "",
            ]
            blocks = []
            for it in buckets[m]:
                body = "\n".join(lines[it["start"]:it["end"] + 1])
                blocks.append(promote_fields(promote_decl(body)))
            for meth in method_buckets[m]:
                body = dedent("\n".join(meth["text"]), meth["indent"])
                # trait impl 的方法不得带可见性限定符（E0449），随 trait 可见性
                if " for " in (meth.get("owner") or ""):
                    blocks.append(body)
                else:
                    blocks.append(promote_decl(body))
            if method_buckets[m]:
                by_owner = {}
                for meth, blk in zip(method_buckets[m], blocks[-len(method_buckets[m]):]):
                    by_owner.setdefault(meth.get("owner") or f"impl {cfg['impl']} {{", []).append(blk)
                blocks = blocks[: len(blocks) - len(method_buckets[m])]
                for owner, blks in by_owner.items():
                    blocks.append(owner + "\n" + "\n\n".join(blks) + "\n}")
            parts.append("\n\n".join(blocks))
            (outdir / f"{m}.rs").write_text("\n".join(parts).rstrip("\n") + "\n")
            print(f"  写入 {outdir / (m + '.rs')}")
        # ---- 重写父文件（仅按区间删除 + 顶部插入声明）----
        drop = set()
        for s, e in moves:
            drop.update(range(s, e + 1))
        rest = [l for idx, l in enumerate(lines) if idx not in drop]
        # 顶部：前导 //! 文档块之后插入 mod/pub use
        k = 0
        while k < len(rest) and (rest[k].strip().startswith("//!") or not rest[k].strip()):
            k += 1
        decls = [f"mod {m};" for m in mods]
        reexports = [f"pub use {m}::*;" for m in mods if buckets[m] or method_buckets[m]]
        new = rest[:k] + [""] + decls + reexports + [""] + rest[k:]
        src.write_text("\n".join(new).rstrip("\n") + "\n")
        print(f"  重写 {src}")
    if not args.apply:
        print("\n（dry-run）")


if __name__ == "__main__":
    main()
