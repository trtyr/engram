# P011 Open Questions

> 只留未决。拍板后移入 decisions/ 并从本文件删除。

## Q001 · 审计事件去向（T001 前置）

`audit()` 直插 jobs 表伪造 succeeded job 行，与 P010「日志唯一时间线」冲突。候选：
- **A. 改走 logs**：audit() 改调 queue emit（target=`audit.<kind>`，fields 带 actor/payload），
  jobs 表不再收伪造行；审计在日志页/MCP logs 可查，与「一切以日志为唯一查看面」自洽。
- **B. 保留现状 + 登记例外**：jobs 面继续承载审计（jobs.list 可查），文档登记
  「审计是 jobs 面的合法内容」。
- 若 A：jobs 表现存审计行如何处置（留停写按 0067 先例回填 logs？还是原地留存）？

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
