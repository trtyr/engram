# P003 Roadmap

> 任务身份/状态/顺序的唯一权威。发现细节权威在 evidence/code-review-2026-10-01.md，此处只追状态。

## In Progress

（空——T001-T003 全部 Done）

## Next

（空）

## Deferred

- **重部署 checklist**（全部修复的生效前置）：生产 engram 重部署一并带上 dd23392（T001 永动机修复）+ 95a115d（EN-11 报错指路）+ 17e5275/72fac41——部署前确认 T001 验收记录在案。

## Done

- [x] **T001 · 敏感判据残留修复**（2026-10-01，`dd23392`）：scenario_converge.rs:81/:116 删 `OR a.sensitive` + core atoms.rs:376 删入队分支 + 4 处过期注释同步 + 回归用例 `sensitive_member_does_not_mark_scenario_stale`（阶段 1 标敏感零 LLM 调用且 updated_at 不动；阶段 2 归档正常触发重算且敏感成员在 refs 保留）。EN 判据面 grep 清零。
- [x] **T002 · Consider 批次 ×7**（2026-10-01，`17e5275` + 档案 v3）：①导出口径注释改「调用方决定」②backup.sh unpack_data 原子交换③hint 截断前 8+总数④守卫分支两用例（非 missing-field 不带指路/键含换行转义固化）+{:?} 必须性注释⑤零命中回显截断 80 字符⑥空白 content 一并拒绝⑦档案《风险与债》v3 P1-4 补「默认排除+reveal 开关」形态说明。
- [x] **T003 · Nit 批次 ×4**（2026-10-01，`17e5275` + `72fac41`）：DEBUG eprintln 残留删除；探针 println 删除（保留宽容解析契约测试）；报错标签「原始类型错误」→「首次严格错误」；backup.sh DSN fallback 加机器专属覆盖说明。
- [x] **附属 · clippy 1.98 存量 expect_used 清零**（2026-10-01，`72fac41`）：api lib 8 处 `.parse().expect("static")` → `HeaderValue::from_static`（clippy 版本漂移暴露的存量 lint，非 P003 引入；门禁阻塞项顺手清）。

## 最终门禁

2026-10-01 `cargo test --workspace` **WORKSPACE_EXIT=0**（73 target ok，PIPESTATUS 捕获，bg_hujnlozk）+ clippy -D warnings（distill/core/mcp/api 全绿）。
