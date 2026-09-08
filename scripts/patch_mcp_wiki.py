# -*- coding: utf-8 -*-
"""MCP lib.rs：wiki 处理器多库穿参 + libraries 操作 + search_all 传库。一次性补丁。"""
p = 'server/crates/mcp/src/lib.rs'
s = open(p, encoding='utf-8').read()

# 0) 解析助手
old = '    /// project_id / project_name 二选一定位项目 id（项目名唯一，可寻址）。\n    async fn resolve_project('
new = (
    '    /// wiki 库 slug → 库 id（缺省 main）。多库（2026-09-08）：所有 wiki 操作按库隔离。\n'
    '    async fn resolve_wiki_lib(&self, library: Option<&str>) -> Result<Uuid, rmcp::ErrorData> {\n'
    '        engram_core::wiki::libraries::resolve(&self.state.pool, library)\n'
    '            .await\n'
    '            .map_err(wiki::from_wiki)\n'
    '    }\n\n'
    '    /// project_id / project_name 二选一定位项目 id（项目名唯一，可寻址）。\n'
    '    async fn resolve_project('
)
assert old in s, 'helper anchor'
s = s.replace(old, new, 1)

subs = []

subs.append((
    "        let wp = params.0;\n        let result = wiki::svc(&self.state)\n            .search_with_purpose(&wp.query, wp.max_items.unwrap_or(20))",
    "        let wp = params.0;\n        let lib = self.resolve_wiki_lib(wp.library.as_deref()).await?;\n        let result = wiki::svc(&self.state)\n            .search_with_purpose(lib, &wp.query, wp.max_items.unwrap_or(20))",
))

subs.append((
    "        let lp = params.0;\n        let pages = wiki::svc(&self.state)\n            .list_pages(\n                lp.page_type.as_deref(),\n                lp.limit.unwrap_or(100),\n                lp.cursor.as_deref(),\n            )",
    "        let lp = params.0;\n        let lib = self.resolve_wiki_lib(lp.library.as_deref()).await?;\n        let pages = wiki::svc(&self.state)\n            .list_pages(\n                lib,\n                lp.page_type.as_deref(),\n                lp.limit.unwrap_or(100),\n                lp.cursor.as_deref(),\n            )",
))

subs.append((
    "        let page = wiki::svc(&self.state)\n            .get_page(&params.0.slug)",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let page = wiki::svc(&self.state)\n            .get_page(lib, &params.0.slug)",
))

subs.append((
    "        let page = wiki::svc(&self.state)\n            .put_page(\n                &wp.slug,",
    "        let lib = self.resolve_wiki_lib(wp.library.as_deref()).await?;\n        let page = wiki::svc(&self.state)\n            .put_page(\n                lib,\n                &wp.slug,",
))

subs.append((
    "        let outcome = wiki::svc(&self.state)\n            .ingest(&wp.title, &wp.text)",
    "        let lib = self.resolve_wiki_lib(wp.library.as_deref()).await?;\n        let outcome = wiki::svc(&self.state)\n            .ingest(lib, &wp.title, &wp.text)",
))

subs.append((
    "        let skipped = wiki::svc(&self.state)\n            .archive_query(&qp.title, &qp.question, &qp.answer)",
    "        let lib = self.resolve_wiki_lib(qp.library.as_deref()).await?;\n        let skipped = wiki::svc(&self.state)\n            .archive_query(lib, &qp.title, &qp.question, &qp.answer)",
))

subs.append((
    "        let graph = wiki::svc(&self.state)\n            .graph()",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let graph = wiki::svc(&self.state)\n            .graph(lib)",
))

subs.append((
    "        let report = wiki::svc(&self.state)\n            .lint()",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let report = wiki::svc(&self.state)\n            .lint(lib)",
))

subs.append((
    "        let deleted = wiki::svc(&self.state)\n            .delete_page(&params.0.slug)",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let deleted = wiki::svc(&self.state)\n            .delete_page(lib, &params.0.slug)",
))

subs.append((
    "        let rows = wiki::svc(&self.state)\n            .page_versions(&params.0.slug)",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let rows = wiki::svc(&self.state)\n            .page_versions(lib, &params.0.slug)",
))

subs.append((
    "        let content = wiki::svc(&self.state)\n            .page_version_content(&params.0.slug, params.0.version)",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let content = wiki::svc(&self.state)\n            .page_version_content(lib, &params.0.slug, params.0.version)",
))

subs.append((
    "        let page = wiki::svc(&self.state)\n            .restore_page_version(&params.0.slug, params.0.version)",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let page = wiki::svc(&self.state)\n            .restore_page_version(lib, &params.0.slug, params.0.version)",
))

subs.append((
    "        let rows = wiki::svc(&self.state)\n            .list_sources()",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let rows = wiki::svc(&self.state)\n            .list_sources(lib)",
))

subs.append((
    "        let report = wiki::svc(&self.state)\n            .delete_source_cascade(id)",
    "        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;\n        let report = wiki::svc(&self.state)\n            .delete_source_cascade(lib, id)",
))

for i, (old, new) in enumerate(subs, 1):
    assert old in s, 'MISS #%d: %s' % (i, old[:70])
    s = s.replace(old, new, 1)

# graph/lint 参数占位换带库版本
old = "_params: Parameters<wiki::WikiNoParams>,"
assert old in s
s = s.replace(old, "Parameters(p): Parameters<wiki::WikiLibParams>,")

# libraries 处理器（挂在待办段注释前）
anchor = "    // ---------- 待办域工具（todos scope；第七域） ----------"
new_fn = (
    "    /// 列出全部 wiki 库（多库；页面/原料计数一并返回）。建库/删库走 Web。\n"
    "    async fn wiki_libraries(\n"
    "        &self,\n"
    "        ctx: RequestContext<RoleServer>,\n"
    "        _params: Parameters<wiki::WikiLibrariesParams>,\n"
    "    ) -> Result<CallToolResult, rmcp::ErrorData> {\n"
    "        let p = principal_of(&ctx)?;\n"
    "        wiki::require_wiki(&p)?;\n"
    "        let rows = engram_core::wiki::libraries::list(&self.state.pool).await;\n"
    "        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))\n"
    "    }\n\n"
)
assert anchor in s
s = s.replace(anchor, new_fn + anchor, 1)

# dispatch 臂
old = (
    '            "delete_source" => {\n'
    "                self.wiki_delete_source(\n"
    "                    ctx,\n"
    '                    Parameters(dispatch::from_args("wiki", "delete_source", call.args)?),\n'
    "                )\n"
    "                .await\n"
    "            }"
)
new = old + (
    '\n            "libraries" => {\n'
    "                self.wiki_libraries(\n"
    "                    ctx,\n"
    '                    Parameters(dispatch::from_args("wiki", "libraries", call.args)?),\n'
    "                )\n"
    "                .await\n"
    "            }"
)
assert old in s, 'dispatch arm'
s = s.replace(old, new, 1)

# search_all wiki 段
old = "            let r = wiki::svc(&self.state).search(&q, max).await;"
new = (
    "            let lib = engram_core::wiki::libraries::resolve(&self.state.pool, None)\n"
    "                .await\n"
    "                .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, e.to_string()))?;\n"
    "            let r = wiki::svc(&self.state).search(lib, &q, max).await;"
)
assert old in s, 'search_all'
s = s.replace(old, new, 1)

open(p, 'w', encoding='utf-8', newline='').write(s)
print('lib.rs patched')
