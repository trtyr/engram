# 代码审查报告 · engram `82fdba7..95a115d`（2026-10-01，全文归档）

> **处置状态（2026-10-01 收官）**：Should-fix-1 ✅ 修复（dd23392，回归用例固化）· Consider ×7 ✅ 全部落地（17e5275 + 档案风险债 v3）· Nit ×4 ✅ 全部清理（17e5275/72fac41）· 附：clippy 1.98 存量 expect_used 8 处清零（72fac41）。最终门禁 workspace EXIT=0（73 target ok）。下文为审查时点原文（reasoning trail）。

> 审查方式：机械层 = fresh-context reviewer 子代理独立审查（全库穷举模式，含宽 pattern 补位）；判断层 = 主 agent 亲手（语义追链 + bde6092 diff 逐行验证 + 突变思维）。
> 方法局限（审查侧失败模式实录）：主 agent 首轮 grep pattern（`NOT sensitive` 字面）存在盲区——判据面以 `OR a.sensitive` 形态存在抓不到；reviewer 宽 pattern `OR\s+(a\.)?sensitive` 补位发现 P1-1。**教训：重构完整性检查的 grep 必须覆盖判据面变体，不能只扫被删字面量。**

## 结论：改后放行（重部署前必须修 Should-fix-1）

## Should-fix-1 · 敏感判据残留一半 → 场景收敛永动机 + 画像隐式剔除复发

**位置**：`server/crates/distill/src/scenario_converge.rs:81`、`:116`、`server/crates/core/src/memory/atoms.rs:376`（失败模式：incomplete refactor——修了 SQL 过滤面漏了判据面）【修复中：P003-T001，2026-10-01】

**证据链**（主 agent 独立验证成立）：

- bde6092 对 scenario_converge.rs 只改 1 行：成员子查询删 `AND NOT a.sensitive`（敏感成员开始写回 atom_refs/members）
- `fetch_stale_scenarios` 判据 `WHERE a.id IS NULL OR a.status != 'active' OR a.sensitive`（:81）未动——含敏感成员的场景仍判 stale
- `collect_removed_texts` 的 `WHERE s.id = $1 AND (a.status != 'active' OR a.sensitive) LIMIT 20`（:116）未动——敏感成员正文每轮被当「已移除表述」收进 removed_texts → persona/prompt.rs:91-95 提示词块 → 画像仍以 sensitive 为剔除条件（与决策 001「标注不再触发隐式级联」冲突）
- 永动机制：敏感成员进 atom_refs → :81 判 stale → 重算 `updated_at=now()` → 下轮再判 stale → 每轮蒸馏白烧 LLM + 20 槽 stale 预算永久占用。改动前循环会终止（一次重算剔出敏感成员），故为 bde6092 引入的行为回归
- `core/src/memory/atoms.rs:376` `|| sensitive == Some(true)` 入队 converge_refresh 分支同理（存量代码被 bde6092 激活）
- 测试缺口：标敏感 → converge 的回归用例零覆盖（全量门禁未抓住的原因）

**修法**：删 :81/:116 两处 `OR a.sensitive`（保留 `a.status != 'active'`）+ 删 atoms.rs:376 分支 + 同步过期注释 ×4（scenario_converge.rs:1/:107、core atoms.rs:374-375、persona/prompt.rs:91-95）+ 补回归用例。无 schema/快照影响。

## Consider ×7

1. 导出侧「默认排除」语义未随决策 001 反转：core/src/memory/ops.rs:201-205 路由层 `unwrap_or(true)` 反转了调用，但服务层注释与 core/tests/memory_test.rs:917-918 仍锁「默认排除」旧口径——改语义或改口径
2. scripts/backup.sh:61-65 unpack_data 先 `rm -rf "$DATA_DIR"` 再解包——备份档损坏时活数据根已删才失败（T001 只修打包侧）；修法：解到 `${DATA_DIR}.new` 再 mv 交换
3. dispatch.rs:295-305 param_name_hint unknown 列表无数量/长度上限——截断前 8 个
4. dispatch.rs:286 `contains("missing field")` 守卫分支零测试覆盖；:303 应注释「必须保持 `{:?}`，改 `{}` 丢转义」
5. project_docs.rs:326（及既有 :388）anchor 回显无长度截断——`old.chars().take(80)`（回显的是调用方自传串且零命中语义自证不在文档中，非泄密，纯长度卫生）
6. project_docs.rs:341 空白 content（`" "`）未拦——等价软删除；拦或文案点明
7. 档案《风险与债》P1-4「已收敛」标注宜补「检索/导出为默认排除+reveal 开关形态」防误读为无条件返回

## Nit ×4

1. project_doc_get_untruncated_test.rs:127-131 DEBUG eprintln 残留（cac3111 引入）
2. dispatch.rs:899 println 探针 + `#[cfg(test)] mod probe_doc_get`（:880）模块残留（基线前 1ad7b597）
3. dispatch.rs:272 文案「原始类型错误：{strict_err}」在 EN-11 场景贴的是 missing field——标签与内容不符（95a115d 照搬）；mcp_param_hint_test.rs:57 反向固化该措辞
4. scripts/backup.sh:11 默认 DSN 写死本机角色 `postgres://trtyr@127.0.0.1:5432/engram`（非凭据；机器专属默认值）

## 已查无问题（明确结论）

- secrets：全库零硬编码 token/key/password；.env.example 全占位符且与 compose 17 变量对齐；CI 全 0 假密钥 ✓
- debug/注释代码：dbg! 零命中；server/crates、deploy、scripts 无注释掉的代码块（仅上述两处 println/eprintln）✓
- correct_atom 守卫（storage atoms.rs:116）有意保留符合决策 001 边界 3，且 core atoms.rs:183 有第二道守卫 ✓
- `NOT sensitive` 参数化开关三处（atoms.rs:206/353、search/hybrid.rs:91）均为「默认排除+reveal/include 开关」设计形态，生产调用面（core/src/memory/search.rs:250,472,332）全传 true——非 T004 漏网 ✓
- bde6092 声明的 7 处移除全部落地（organize/entity_portraits/consolidate×3/ops timeline 零 sensitive 过滤命中）✓
- 三份新测试（replace_text 5 用例/untruncated 1 用例/param_hint 2 用例）突变敏感：撤修复即红，无同义反复 ✓
- param_name_hint 只回显键名不回显值；`{:?}`（escape_debug 语义）转义换行/控制字符——日志注入面关闭 ✓
- replace_text 报错不回显文档内容（零命中语义自证）、不回显 new_text ✓
- golden mcp_surface.json 未动 ✓（95a115d 零 schema 变更）

## 审查方法备注

- reviewer 子代理无 diff 产物，报告为 HEAD 现状实证 + reflog 区间确认（7 commits 拓扑与题述一致）；工作树对 HEAD 干净（未跟踪仅 AGENTS.md/docs/plantree/，决策 002 口径）
- 待运行验证（修复 T001 时一并执行）：`cargo test -p engram-distill -p engram-core -p engram-storage`（验证删 `OR a.sensitive` 后无被测契约翻红）→ 全量 workspace
- 建议补断言：含 `\n` 的非法键名送 from_args，断言错误串无裸换行（固化转义结论）
