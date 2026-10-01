# Decision 001 · sensitive 降级为纯标记（统一放开）

- **日期**：2026-09-28（同日边界三项拍板，已定案）
- **拍板人**：trtyr（原话：「我不在意敏感的东西，你就根据我这个思想去搞吧」+ 边界拍板「导出迁移包也要带这个敏感原子。后面两个你自己决定」「002，可以，跑进画像里」）
- **状态**：✅ 定案

## Context

2026-09-12 拍板「敏感内容放开检索」（core/src/memory/search.rs:98-100 移除 reveal 门槛），但蒸馏侧的排除逻辑全部保留：organize 取原子 `WHERE NOT sensitive`（organize.rs:82-88）、converge 活跃成员判定（scenario_converge.rs:80-90）、实体档案（entity_portraits.rs:47-50）、consolidate 聚类与关系回溯、timeline（ops.rs:66-70）、export 默认排除。同一原子「检索可见、记忆不可见」，且标注动作会隐式触发蒸馏级联（场景解散/画像清退）。

## Decision

**sensitive 从「蒸馏排除条件」降级为「纯标记」**：保留字段与标注 UI（可筛选、可审计），但蒸馏链全链路（organize/scenarios/persona/converge/entity_portraits/consolidate/timeline）不再以 sensitive 为过滤条件。

## Consequences

- ✅ 消除语义分裂：标注不再触发隐式级联；检索/记忆口径一致
- ✅ 删除面：organize/converge/portraits/consolidate/ops 的 WHERE 条件收紧，测试同步

### 边界定案（2026-09-28 三项）

1. **导出/迁移包带敏感原子**：`/memory/export` 默认含敏感（`include_sensitive` 语义反转或移除）——换机/备份场景敏感数据随迁
2. **deep purge 不受影响**：物理清空无敏感概念（确认项）
3. **correct_atom 快路径「敏感禁走」保留**（Mia 决定，纵深防御）：敏感原子不走 Admin confidence 0.95 直转快路径，仍须走蒸馏取代链；主放开方向不受影响

## Consequences（原判）

- ⚠️ 敏感内容将进入 L2/L3 并参与 prompt 注入——与「用户不在意敏感」的定位一致
- ⚠️ 存量回填：跑 `distill {mode:"rebuild"}` 把历史被排除的敏感原子补进场景/画像（Q002 拍板：跑；目标实例待确认——生产 vs 本机）

## Alternatives

- 反向收敛（检索重新排除敏感）：与 2026-09-12 用户拍板相反，否决
- 维持现状双口径：分裂持续扩大，否决

## Related

- 档案：风险与债 P1-4 · 蒸馏链篇「坑与疑点」#1 · ADR D4
- 落地任务：T004（Deferred，等 Q001/Q002）
