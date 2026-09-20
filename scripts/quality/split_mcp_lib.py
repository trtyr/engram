#!/usr/bin/env python3
"""mcp/src/lib.rs 按域拆分生成器（纯搬移，零行为变化）。

用法：
    python3 scripts/quality/split_mcp_lib.py --dry [--names]   # 只看归类
    python3 scripts/quality/split_mcp_lib.py --apply           # 写文件

设计（2026-09-20 架构治理 task-2）：
- lib.rs 的顶层项与 `#[tool_router] impl EngramMcpServer` 块内的方法按名归类到各域模块；
- 每个域模块自带 `#[tool_router(router = <mod>_router)]`，并导出 `routes_<mod>()` 包装，
  由装配层合并（rmcp 3.2 的 ToolRouter::merge）；
- 每个生成模块以 `use super::*;` 继承根模块导入与再导出，避免逐模块 import 手术；
- jobs.rs / wiki.rs 已存在（早前抽出），新增内容追加而非覆盖。
"""
import argparse
import re
from pathlib import Path

SRC = Path("server/crates/mcp/src/lib.rs")
TOP = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|struct|enum|trait|impl|const|static|type|use|mod|macro_rules!)\b"
)
METHOD = re.compile(r"^    (?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)")
NAME = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|struct|enum|trait|const|static|type|mod)\s+([A-Za-z0-9_]+)"
)

EXISTING = {"jobs"}  # 已存在的模块文件：追加（wiki.rs 是既有的参数模块，不动它）

# 先匹配者胜。大写开头 = 类型名（CamelCase），小写 = 函数/常量（snake_case）。
RULES = [
    # ---- search_all 最先（否则被 memory 的 ^search 抢走）----
    (r"^search_all", "search_all"), (r"^SearchAll", "search_all"),
    # ---- projects 子域 ----
    (r"^project_loc", "project_locations"), (r"^ProjectLocation", "project_locations"),
    (r"^project_doc", "project_docs"), (r"^ProjectDoc", "project_docs"),
    (r"^doc_", "project_docs"), (r"^Doc", "project_docs"),
    (r"^project_file", "project_files"), (r"^ProjectFile", "project_files"),
    (r"^file_", "project_files"), (r"^File", "project_files"),
    (r"^project", "projects"), (r"^Project", "projects"),
    # ---- wiki 子域 ----
    (r"^wiki_document", "wiki_docs"), (r"^wiki_docs", "wiki_docs"), (r"^wiki_ingest", "wiki_docs"),
    (r"^WikiDocument", "wiki_docs"), (r"^WikiDocs", "wiki_docs"), (r"^WikiIngest", "wiki_docs"),
    (r"^wiki_(lint|merge|reviews|review_resolve|insights|promote|promotions|sources|delete_source|versions|version_content|restore_version)",
     "wiki_curation"),
    (r"^Wiki(Lint|Merge|Review|Insights|Promote|Source|Version|Restore)", "wiki_curation"),
    (r"^wiki", "wiki_ops"), (r"^Wiki", "wiki_ops"), (r"^Lib", "wiki_ops"),
    # ---- memory 子域 ----
    (r"^memory_kv", "memory_kv"), (r"^MemoryKv", "memory_kv"), (r"^Kv", "memory_kv"),
    (r"^memory_(list_sessions|get_session|write_session|append_session|forget|scenario)", "memory_sessions"),
    (r"^memory_(remember|correct|confirm|discard|persona_edit|distill|entities)", "memory_write"),
    (r"^(Remember|Correct|ReviewAction|PersonaEdit|Distill|Entities)", "memory_write"),
    (r"^(ListSessions|GetSession|WriteSession|AppendSession|Forget|Scenario)", "memory_sessions"),
    (r"^memory", "memory"), (r"^Memory", "memory"),
    (r"^(Context|Search|ListAtoms|Turn)", "memory"),
    # ---- 其余域 ----
    (r"^skills_(versions|restore|delete)", "skills_versions"),
    (r"^Skills(Versions|Restore|Delete)Params", "skills_versions"),
    (r"^skill", "skills"), (r"^Skill", "skills"),
    (r"^todo_(link|unlink|links|ref_id)", "todos_links"),
    (r"^Todo(Link|Unlink|Links)Params", "todos_links"),
    (r"^todo", "todos"), (r"^Todo", "todos"),
    (r"^ticket", "tickets"), (r"^Ticket", "tickets"),
    (r"^(codegraph|cg_)", "codegraph"), (r"^(Codegraph|Cg)", "codegraph"),
    (r"^job", "jobs"), (r"^Job", "jobs"),
]

GUARD_NAMES = {
    "mcp_err", "ok_json", "slim_content", "strip_keys", "parse_flex_datetime",
    "principal_of", "require_memory", "require_original", "require_project",
    "require_erase", "require_skills", "require_todos", "require_codegraph",
    "from_memory", "from_project", "from_skills", "from_todo", "from_cg",
    "slim_doc", "slim_session", "slim_skill", "slim_todo", "slim_atom",
}
REGISTRY_NAMES = {
    "tool_scope", "ToolCatalogs", "CATALOG_ITEM_CAP", "SERVER_INSTRUCTIONS",
    "McpConfig", "MCP_SETTINGS_KEY", "load_config", "save_config", "gate",
    "default_true", "with_dynamic_description", "McpActionInfo", "McpToolInfo",
    "McpInfo", "tool_catalog", "build_info", "is_domain_tool",
}
SERVER_NAMES = {"EngramMcpServer", "service", "Turn"}
IMPL_MAP = {
    "impl:EngramMcpServer {": "server",
    "impl:ServerHandler for EngramMcpServer {": "server",
    "impl:ToolCatalogs {": "registry",
    "impl:Default for McpConfig {": "registry",
}
EXTRA = {
    "EntitiesParams": "memory",
    "parse_job_statuses": "jobs",
    "from_job": "jobs",
    "from_wiki_docs": "wiki_ops",
}


def classify(name: str) -> str:
    for table in (GUARD_NAMES, REGISTRY_NAMES, SERVER_NAMES):
        if name in table:
            return {"mcp_err": "guard"}.get(name, name in SERVER_NAMES and "server"
                                            or name in REGISTRY_NAMES and "registry" or "guard")
    if name in GUARD_NAMES:
        return "guard"
    if name in REGISTRY_NAMES:
        return "registry"
    if name in SERVER_NAMES:
        return "server"
    if name in IMPL_MAP:
        return IMPL_MAP[name]
    if name in EXTRA:
        return EXTRA[name]
    for pat, mod in RULES:
        if re.match(pat, name):
            return mod
    return "UNKNOWN"


def _strip_strings(line):
    return re.sub(r'"(?:[^"\\]|\\.)*"', "", line)


def split_top_level(lines):
    """顶层项切分：属性/注释先入缓冲，遇关键字行则配平到项尾，缓冲一并归属。

    比「找下一个关键字行」稳：空行与段注释不会把属性错配给相邻项。
    """
    items, pending, i, n = [], [], 0, len(lines)
    while i < n:
        raw = lines[i]
        st = raw.strip()
        if not st or st.startswith("#[") or st.startswith("//"):
            pending.append(raw)
            i += 1
            continue
        if TOP.match(raw.rstrip()):
            kw = kind_of(raw)
            if kw in ("use", "mod", "const", "static", "type"):
                # 无花括号项：只在「行尾是分号且非续行」处结束——多行字符串（行尾 \）里
                # 可能含 { } 示例（如 {"action":"help"}），绝不能按花括号配平。
                k = i
                while k < n:
                    line = lines[k].rstrip()
                    if line.endswith(";"):
                        break
                    k += 1
            else:
                k, depth, started = i, 0, False
                while k < n:
                    c = _strip_strings(lines[k])
                    depth += c.count("{") - c.count("}")
                    if "{" in c:
                        started = True
                    if started and depth <= 0:
                        break
                    k += 1
            items.append({"text": pending + lines[i:k + 1], "kind": kind_of(raw), "name": name_of(raw)})
            pending = []
            i = k + 1
            continue
        pending.append(raw)  # 兜底：未知行并入缓冲
        i += 1
    if pending:
        if items:
            items[-1]["text"] += pending
        else:
            items.append({"text": pending, "kind": "?", "name": "?"})
    return items


def kind_of(l):
    for kw in ("fn", "struct", "enum", "trait", "impl", "const", "static", "type", "use", "mod", "macro_rules!"):
        if re.match(rf"^(?:pub\S*\s+)?(?:async\s+)?{re.escape(kw)}\b", l):
            return kw
    return "?"


def name_of(l):
    m = NAME.match(l)
    if m:
        return m.group(1)
    m = re.match(r"^(?:pub\s+)?(?:unsafe\s+)?impl\b\s*(.*)$", l)
    return ("impl:" + m.group(1)[:39]) if m else "?"


def split_methods(block):
    """impl 块内方法切分：属性/注释缓冲 + 花括号配平（同顶层逻辑，缩进 4）。"""
    methods, pending = [], []
    i, n = 1, len(block) - 1
    while i < len(block):
        if re.match(r"^(?:pub\s+)?(?:unsafe\s+)?impl\b", block[i]):
            i += 1
            break
        i += 1
    while i < n:
        raw = block[i]
        st = raw.strip()
        if not st or st.startswith("#[") or st.startswith("//") or st == "}":
            pending.append(raw)
            i += 1
            continue
        if METHOD.match(raw):
            k, depth, started = i, 0, False
            while k < n:
                c = _strip_strings(block[k])
                depth += c.count("{") - c.count("}")
                if "{" in c:
                    started = True
                if started and depth <= 0:
                    break
                k += 1
            methods.append({"text": pending + block[i:k + 1], "name": METHOD.match(raw).group(1)})
            pending = []
            i = k + 1
            continue
        pending.append(raw)
        i += 1
    if pending and methods:
        methods[-1]["text"] += pending
    return methods


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


def dedent(text, n=4):
    return "\n".join(l[n:] if l.startswith(" " * n) else l for l in text.split("\n"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry", action="store_true")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--names", action="store_true")
    args = ap.parse_args()

    lines = SRC.read_text().split("\n")
    head_end = 0
    for i, l in enumerate(lines):
        if TOP.match(l.rstrip()):
            head_end = i
            break
    header = lines[:head_end]

    top: dict[str, list[str]] = {}
    router_items: dict[str, list[dict]] = {}
    unknown: list[str] = []

    for item in split_top_level(lines[head_end:]):
        if item["kind"] in ("use", "mod"):
            top.setdefault("_root", []).append("\n".join(item["text"]))
            continue
        if item["kind"] == "impl" and any(
            l.strip().startswith("#[tool_router]") for l in item["text"][:8]
        ):
            for m in split_methods(item["text"]):
                dest = classify(m["name"])
                if dest == "UNKNOWN":
                    unknown.append(m["name"])
                    dest = "memory"
                router_items.setdefault(dest, []).append(m)
            continue
        dest = classify(item["name"])
        if dest == "UNKNOWN":
            unknown.append(item["name"])
            dest = "_root"
        top.setdefault(dest, []).append("\n".join(item["text"]))

    def size(k):
        return code_lines("\n".join(top.get(k, []))) + code_lines("\n".join("\n".join(m["text"]) for m in router_items.get(k, [])))

    mods = sorted(set(list(top) + list(router_items)) - {"_root"}, key=lambda k: -size(k))
    print(f"{'模块':16} {'顶层项':>6} {'router 方法':>11} {'≈代码行':>8}")
    for k in mods:
        print(f"{k:16} {len(top.get(k, [])):6d} {len(router_items.get(k, [])):11d} {size(k):8d}")
    print(f"{'_root(留 lib)':16} {len(top.get('_root', [])):6d} {0:11d} {code_lines(chr(10).join(top.get('_root', []))):8d}")
    if unknown:
        print(f"\n未归类 {len(unknown)}: {unknown}")
    if args.names:
        for k in mods:
            print(f"\n[{k}] {size(k)} 代码行")
            print("  方法:", ", ".join(m["name"] for m in router_items.get(k, [])))
    if not args.apply:
        print("\n（dry-run）")
        return

    out = SRC.parent
    for k in mods:
        body_items = top.get(k, [])
        methods = router_items.get(k, [])
        first = (
            f"//! {k} 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。\n"
            if k not in EXISTING
            else f"// --- {k} 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）---\n"
        )
        parts = [first, "use super::*;\n"]
        if k.startswith("wiki"):
            parts.append("use crate::wiki::*;  // 该域参数与错误桥仍在既有的 wiki 模块\n")
        if body_items:
            parts.append("\n".join(body_items) + "\n")
        if methods:
            parts.append(f"#[tool_router(router = {k}_router)]\nimpl EngramMcpServer {{")
            parts.append("\n\n".join(dedent("\n".join(m["text"])) for m in methods))
            parts.append("}\n")
            parts.append(
                f"/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。\n"
                f"pub(crate) fn routes_{k}() -> ToolRouter<EngramMcpServer> {{\n"
                f"    EngramMcpServer::{k}_router()\n}}\n"
            )
        content = "\n".join(parts)
        path = out / f"{k}.rs"
        if k in EXISTING:
            path.write_text(path.read_text().rstrip("\n") + "\n\n" + content)
        else:
            path.write_text(content)
        print(f"写入 {path} ({len(content.splitlines())} 行)")

    root = top.get("_root", [])
    mod_decls = [l for l in root if l.strip().startswith(("mod ", "pub mod "))]
    uses = [l for l in root if l not in mod_decls]
    new_mods = [f"mod {k};" for k in mods if k not in EXISTING]
    ext_mods = [l for l in mod_decls if any(k in l for k in EXISTING)]
    pub_mods = [l for l in mod_decls if l.strip().startswith("pub mod") and not any(k in l for k in EXISTING)]
    merges = "\n".join(f"        tool_router.merge(crate::{k}::routes_{k}());" for k in mods if router_items.get(k))
    lib = "\n".join([
        "\n".join(header).rstrip("\n"),
        "",
        *pub_mods,
        *new_mods,
        *ext_mods,
        "",
        "// 架构治理 2026-09-20：lib.rs 只留 crate 装配（类型/构造/router 合并/再导出），",
        "// 各域工具面在各自模块内（`#[tool_router(router = <mod>_router)]` + `routes_<mod>()`）。",
        # 再导出：保持 `crate::X` 路径对既有模块（dispatch/wiki/jobs）与各域模块可见。
        "pub use registry::*;",
        "pub use server::*;",
        *[f"pub(crate) use {k}::*;" for k in mods if k not in ("registry", "server")],
        "",
        *uses,
        "",
        "impl EngramMcpServer {",
        "    /// 合并各域工具 router（每个域模块自带 `#[tool_router]` 块）。",
        "    pub(crate) fn build_tool_router() -> ToolRouter<Self> {",
        "        let mut tool_router = ToolRouter::<Self>::new();",
        merges,
        "        tool_router",
        "    }",
        "}",
        "",
    ])
    (out / "lib.rs").write_text(lib)
    print(f"\n重写 lib.rs（{len(lib.splitlines())} 行）")


if __name__ == "__main__":
    main()
