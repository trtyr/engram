# -*- coding: utf-8 -*-
"""ZZTEST-M10：Wiki 多库黑盒测试载具（裸 JSON-RPC + HTTP 直连 127.0.0.1）。"""
import json, time, urllib.request, urllib.error

BASE = "http://127.0.0.1:8080"
M = "zztest-m10"  # 本轮标记

def http(method, path, body=None, token=None):
    r = urllib.request.Request(BASE + path, data=json.dumps(body).encode() if body is not None else None, method=method)
    r.add_header("Content-Type", "application/json")
    r.add_header("Accept", "application/json, text/event-stream")
    if token: r.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(r, timeout=30) as resp:
            raw = resp.read().decode()
            return resp.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, raw

TOK = http("POST", "/auth/login", {"password": "admin123"})[1]["token"]
RPC_ID = [0]
SESSION = []

def rpc_init():
    body = {"jsonrpc": "2.0", "id": RPC_ID[0], "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "zztest-m10", "version": "0"}}}
    st, v = http("POST", "/mcp", body, TOK)
    assert st == 200, st

def mcp(tool, action=None, **args):
    arguments = dict(args)
    if action is not None:
        arguments["action"] = action
    RPC_ID[0] += 1
    body = {"jsonrpc": "2.0", "id": RPC_ID[0], "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}}
    st, v = http("POST", "/mcp", body, TOK)
    if "error" in v:
        return {"__error__": v["error"]["message"]}
    out = v["result"]["content"][0]["text"]
    try:
        return json.loads(out)
    except Exception:
        return {"__raw__": out}

def wiki(action=None, **args):
    return mcp("wiki", action, **args)

PASS, FAIL = [], []
def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else "  -> " + str(detail)[:300]))

# ========== A. 库管理面 ==========
libs = wiki("libraries")
check("A1 libraries 列出 main", isinstance(libs, list) and any(l["slug"] == "main" for l in libs), libs)

r = wiki("libraries")
n0 = len(r)
r = wiki("libraries")  # 占位
st, v = http("POST", "/wiki/libraries", {"slug": M, "name": "多库黑盒测试库"}, TOK)
check("A2 HTTP 建库", st == 200 and v["slug"] == M, v)
st2, v2 = http("POST", "/wiki/libraries", {"slug": M, "name": "重复"}, TOK)
check("A3 重复建库 400", st2 == 400, (st2, v2))
st3, v3 = http("POST", "/wiki/libraries", {"slug": "大写BAD", "name": "x"}, TOK)
check("A4 非法 slug 400", st3 == 400, (st3, v3))

libs = wiki("libraries")
mine = next((l for l in libs if l["slug"] == M), None)
check("A5 MCP libraries 可见新库且计数 0", mine is not None and mine["pages"] == 0 and mine["sources"] == 0, libs)

r = wiki("libraries")
# ========== B. 同名 slug 跨库隔离（页面全生命周期） ==========
r = wiki("write_page", slug="zz-m10-shared", title="双库靶页", content="主库版本：" + M)
check("B1 write_page 主库", "content_chars" in r, r)
r = wiki("write_page", slug="zz-m10-shared", library=M, title="双库靶页", content="测试库版本：" + M)
check("B2 write_page 测试库", "content_chars" in r, r)

r = wiki("get_page", slug="zz-m10-shared")
r2 = wiki("get_page", slug="zz-m10-shared", library=M)
check("B3 get_page 同名不同内容", r["content"] != r2["content"] and "主库版本" in r["content"] and "测试库版本" in r2["content"], (r.get("content"), r2.get("content")))

# 各自独立版本史：主库再覆盖一次 → 测试库版本数不变
wiki("write_page", slug="zz-m10-shared", title="双库靶页", content="主库版本2：" + M)
v_main = wiki("versions", slug="zz-m10-shared")
v_test = wiki("versions", slug="zz-m10-shared", library=M)
check("B4 版本史按库隔离（主库有历史、测试库零历史）", len(v_main) >= 1 and len(v_test) == 0, (len(v_main), len(v_test)))

# 检索按库
r = wiki("search", query="测试库版本")
check("B5 主库检索不中测试库内容", not any(p["slug"] == "zz-m10-shared" for p in r.get("pages", [])), r.get("pages"))
r = wiki("search", query="测试库版本", library=M)
check("B6 测试库检索命中", any(p["slug"] == "zz-m10-shared" for p in r.get("pages", [])), r.get("pages"))

# 图谱按库（两张同名页各自成节点）
g0 = wiki("graph", library=M)
check("B7 测试库图谱只有自己的页", any(n["slug"] == "zz-m10-shared" for n in g0.get("nodes", [])) and len([n for n in g0.get("nodes", []) if n["slug"] == "zz-m10-shared"]) == 1, g0.get("nodes"))

# lint 按库
l0 = wiki("lint", library=M)
check("B8 lint 按库", "checked_pages" in l0, l0)

# purpose 每库
wiki("write_page", slug="zz-m10-p", title="p", content="x", library=M)
# purpose 走 HTTP（MCP 无 purpose 操作）
st, pm = http("GET", "/wiki/purpose", None, TOK)
st2, pt = http("GET", "/wiki/purpose?lib=" + M, None, TOK)
http("PUT", "/wiki/purpose?lib=" + M, {"goals": ["黑盒测试目标"], "key_questions": [], "scope": []}, TOK)
st3, pt2 = http("GET", "/wiki/purpose?lib=" + M, None, TOK)
st4, pm2 = http("GET", "/wiki/purpose", None, TOK)
check("B9 purpose 每库独立", pt2.get("goals") == ["黑盒测试目标"] and (pm2 is None or "黑盒测试目标" not in (pm2.get("goals") or [])), (pm2, pt2))

# ========== C. 织入库传播 ==========
r = wiki("ingest", library=M, title=M + " 织入原料", text="霞鹭文楷是一种中文开源字体，由开发者社区维护，常用于终端与阅读器。")
check("C1 ingest 入队带库", r.get("status") == "enqueued" and r.get("source_id"), r)
src_id = r.get("source_id")
# 轮询最多 ~90s 等织入完成（LLM 流水线）
done = False
for _ in range(70):
    time.sleep(3)
    st, jobs = http("GET", "/jobs", None, TOK)
    rows = jobs if isinstance(jobs, list) else jobs.get("jobs", jobs)
    try:
        rel = [j for j in rows if isinstance(j, dict)
               and isinstance(j.get("payload"), dict)
               and j["payload"].get("source_id") == str(src_id)]
        if rel and all(j.get("status") in ("succeeded", "failed", "dead") for j in rel):
            done = True
            break
    except Exception:
        pass
check("C2 织入任务完成", done, rows if not done else "")

srcs = wiki("sources", library=M)
check("C3 原料挂在测试库", any(s["source_id"] == src_id for s in srcs), srcs)
r = wiki("list_pages", library=M)
check("C4 织入管线进测试库（index 系统页落库；薄内容允许 0 产物）", any(p["slug"] == "index" for p in r), [p["slug"] for p in r])

# ========== D. 边角与负面 ==========
r = wiki("write_page", slug="zz-m10-neg", library="不存在的库", title="x", content="x")
check("D1 未知库写页报错", "__error__" in r and ("不存在" in r["__error__"] or "no such" in r["__error__"].lower() or "NotFound" in r["__error__"]), r)
r = wiki("ingest", library="不存在的库", title="x", text="x")
check("D2 未知库织入报错", "__error__" in r, r)
r = wiki("search", query="x", library="不存在的库")
check("D3 未知库检索报错", "__error__" in r, r)

# 未知库上的删除/版本不误伤主库同名页
r = wiki("delete_page", slug="zz-m10-shared", library="不存在的库")
check("D4 未知库删页报错", "__error__" in r, r)
r = wiki("get_page", slug="zz-m10-shared")
check("D5 主库同名页仍完好", "content" in r and M in r["content"], r)

# main 保护（HTTP 删除主库）
st, v = http("DELETE", "/wiki/libraries/main", None, TOK)
check("D6 main 库删除被拒", st == 400, (st, v))

# ========== 清场 ==========
print("\n== 清场 ==")
# 删测试库（force 级联：页面/原料/织入产物全走）
st, v = http("DELETE", "/wiki/libraries/" + M + "?force=true", None, TOK)
check("Z1 force 删测试库", st == 200, (st, v))
st, _ = http("GET", "/wiki/pages?lib=" + M, None, TOK)
check("Z2 已删库 404", st == 404, st)
# main 靶页删除
for slug in ["zz-m10-shared", "zz-m10-p"]:
    st, _ = http("DELETE", "/wiki/pages/" + slug + "?lib=main", None, TOK)
    print("  cleanup", slug, st)
# main 的织入原料（若 C1 意外落到 main）
srcs_main = wiki("sources")
for s in srcs_main if isinstance(srcs_main, list) else []:
    if M in (s.get("title") or ""):
        http("DELETE", "/wiki/sources/" + s["source_id"], None, TOK)
        print("  cleanup source", s["source_id"])
libs = wiki("libraries")
check("Z3 库列表回基线", all(l["slug"] == "main" for l in libs), libs)

print(f"\n=== {len(PASS)}/{len(PASS) + len(FAIL)} PASS ===")
if FAIL:
    print("FAILURES:", FAIL)
