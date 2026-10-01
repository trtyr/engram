# P003 · 代码审查发现修复线

> 来源：2026-10-01 code-review（code-review 技能五步流程）——审查范围 engram `82fdba7..95a115d`（7 commits，全部 AI 生成代码：P001 收尾 4 + P002 修复 3）。
> 审查报告全文（reasoning trail）：[evidence/code-review-2026-10-01.md](evidence/code-review-2026-10-01.md)；本树只追修复状态。

## Scope

**In**：审查发现 P1-1（敏感判据残留→场景收敛永动机，Should-fix/重部署前必修）、Consider ×7、Nit ×4 的修复与验证。
**Out**：ChungusHub 工单（别处台账）；重设计/重构；P001 档案债 22 项（另一线）。

## Affected Modules

`server/distill` `server/core` `server/storage` `server/mcp` `scripts` `docs-repo`（档案《风险与债》P1-4 标注补形态说明）

## 文件地图（阅读路径）

1. [roadmap.md](roadmap.md) — 任务状态（唯一任务权威）
2. [evidence/code-review-2026-10-01.md](evidence/code-review-2026-10-01.md) — 审查报告全文（发现明细/证据链/已查无问题清单）
3. [open-questions.md](open-questions.md) — 等你拍板的

## 执行纪律

- **T001 是重部署前置项**：生产 engram 重部署（含 EN-11 修复 95a115d）前必须先落 T001，否则敏感判据永动机上线（每轮蒸馏白烧 LLM + 画像隐式剔除复发）
- 修 P1-1 必须补回归用例（标敏感 → 不再触发 converge——现测试零覆盖，正是全量门禁漏掉它的原因）
- 门禁照旧：cargo test --workspace（PIPESTATUS 真退出码）+ clippy -D warnings
- 修复后本 README/roadmap 记 commit 证据；发现条目关闭时在 evidence 对应条目标 ✅
