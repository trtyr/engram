# P011 Open Questions

> 只留未决。拍板后移入 decisions/ 并从本文件删除。

## Q001 · 审计事件去向——**已拍板（2026-10-03）：改走 logs**

用户原话：「我们现在系统里没有 job 这个东西了，只有日志。」——审计是记忆域的
治理记录，归 logs 流。落地：T001 实现时 `audit()` 改走 emit（target=`audit.<kind>`，
fields 带 actor/payload），jobs 表不再收伪造 succeeded 行；历史审计行是否回填
随实现评估（量小，倾向不回填、原地留存）。

## Q002 · revive_entity 处置（T008/T021 前置）——**已改判（2026-10-03）**

原悬「删函数 vs 接线」。按用户架构定义「记忆与圈子同联」（T021）：归档实体被再次
提及应**复活延续档案**（summary/修订史保全），而非新建空壳造成档案分裂——
**改判：恢复接线**（link_entity 查归档档并 revive），随 T021 ①落地。

## Q003 · 场景成员双轨修复策略（T003 前置）

- **A. scenario_id 单轨化**：atoms.scenario_id 为唯一真源，scenarios.atom_refs 降级为
  快照缓存（organize 写、converge 对齐 scenario_id 重算）——查询侧（entity_scenarios/
  timeline）已用 scenario_id，改动集中在 converge 与 organize 写侧。
- **B. 保持双轨 + 一致性校验**：加一次性校验 SQL（定期对账 refs vs scenario_id），
  漂移靠 converge 自然修复，不动结构。
- **C. atom_refs 单轨化**：反向——影响面最大（converge/organize/场景检索全要改），不推荐。

## Q004 · forget_entity 的物理删是否符合治理哲学（低优先）

atoms 治理红线「归档不删除」，但 forget_entity（core/entity.rs:285）物理 DELETE 实体
（仅留原子归档）。实体档案（summary/revision）随之蒸发。要不要统一为实体也可归档？
当前无工单驱动，登记备查。
