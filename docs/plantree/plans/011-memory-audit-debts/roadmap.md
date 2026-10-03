# P011 Roadmap

> 任务身份/状态/顺序的唯一权威。全部 Planning——每条需拍板后进 In Progress。
> 每条带实读证据（文件:行号，2026-10-03 实读）。

## Done

（空）

## In Progress

（空）

## Next（待拍板，按建议优先级）

### 正确性

- [ ] **T001 · 审计事件不进 logs 流**【Q001】
  repo/memory/ops.rs:88 `audit()` 直插 jobs 表伪造 `status='succeeded'` 的 job 行
  （注释自认「写一条已完成的 job 行」）。调用点 11+：session_void_cascade /
  session_erase_cascade / session_unvoid_restore / correct_atom / edit_atom /
  review_confirm / review_discard / atom_archive / kv_delete / entity_merge /
  edit_persona / delete_entity。**与 P010「日志是唯一时间线」原则冲突**——审计只在
  MCP jobs 域可见，logs 流与前端日志页看不到。
- [ ] **T002 · mark_superseded 双 active**
  distill/arbitrate.rs:296 `WHERE id=$2 AND status='active'`——旧条已 archived 时
  UPDATE 0 行但候选已转正（:235 先 promote 再 supersede），同主题双 active。
  蒸馏链篇坑清单第 5 条实证。
- [ ] **T003 · 场景成员双轨漂移**
  写侧：distill/organize.rs:270 `update_scenario` 的 atom_refs 是**并集**（旧∪新，
  只进不出）；:317 `refresh_embeddings` 的 atoms.scenario_id 是**覆盖回填**。
  原子被场景 B update 拉走时：A.atom_refs 残留 + scenario_id 改指 B。
  读侧分裂：entity_scenarios（repo/entity.rs:67）/timeline 用 scenario_id 正查，
  converge（scenario_converge.rs:75）用 atom_refs 反查——两轴一致性仅靠 converge 兜底。
- [ ] **T004 · update_atom_full 单值字段清不掉**
  repo/memory/atoms.rs:271-274 superseded_by/occurred_at/valid_until/sensitive 全
  `COALESCE($X, 保留旧值)`——传 None 永远无法置回 NULL。EN-BUG-1（study learned_at）
  同族盲 CASE，方向相反：那边保不住、这边清不掉。
- [ ] **T005 · 实体子串归并误吞 + 关系 lookup 口径分裂**
  distill/extract.rs:226 `position(lower($1) in lower(name))>0 OR 反向` 双向包含
  + `LIMIT 1` 无 ORDER BY——「云」可吞「星云」；:298 关系 lookup 用 `name=$1` 精确
  匹配，与挂链的子串归并口径不一致。坑清单第 3 条实证（P001 Deferred 只挂了
  ORDER BY 半条，双向包含与口径分裂在此线补全）。

### 性能

- [ ] **T006 · extract claim 全量抢占无 LIMIT**
  distill/extract.rs:46 `UPDATE raw_sessions SET distill_status='processing'
  WHERE distill_status='pending' AND ... RETURNING ...`——一次认领全部 pending，
  无 LIMIT。会话量级增长后第一个慢查询（坑清单第 2 条实证）。
- [ ] **T007 · LLM 无熔断**
  distill/llm_port.rs budget_tokens 声明未用——蒸馏风暴无成本闸门（坑清单第 4 条）。

### 卫生

- [ ] **T008 · revive_entity 死代码 + 永不复活后果**【Q002】
  repo/memory/entity.rs:459 全仓零调用；extract.rs:264 注释明示「复活语义已废除」。
  实际后果：孤儿实体归档后即使同名再次出现也**新建实体**（link_entity 只查活体），
  旧档案（summary/revision）永沉归档态。删函数或恢复接线，二选一。
- [ ] **T009 · stale 注释三处（决策 001 后未跟上）**
  ① search/hybrid.rs:86「P3：sensitive 原子默认排除」——主检索路径已全传 true
  （core/search.rs:250/472/332）；② core/memory/search.rs:426 F4 注释「归档或标敏感
  →收敛」——实际仅归档触发（:377，标敏感不触发是 P003-T001 决策）；③
  repo/memory/atoms.rs:368 recent_active_atoms 注释「过滤过期与敏感」——SQL 只滤过期。
- [ ] **T010 · mcp_test.rs:446 文案滞后**
  报错文案「应为十二个域工具」vs 断言 `tools.len()==13`（P010 加 logs 域后没跟）。

### 能力候选（2026-10-03 L0→L1 机制讨论产生，待拍板排序）

- [ ] **T011 · 蒸馏归因回执（段级判定可观测）**
  extract 每段产出结构化回执：抽出 N 条候选 / 显式拒绝 + 理由短语——落任务事件流，
  抽取率（拒收段占比）可统计。动机：误杀（相关被判无关）是沉默丢失，现在只有一句
  「无持久洞察」日志；后续调 prompt v9+、评估保险级联全靠这个数据。背景：extract
  prompt 规则 8 已要求「不值得记输出空数组.宁缺毋滥」（prompts.rs:40），但拒绝原因
  不落盘。
- [ ] **T012 · JEV 决策模型适配评估（openrouter typesafe/jev-1.13）**
  System One 决策模型（choice/noul/score 三原语，typed 输出+概率，输出 token 免费，
  $0.042/M 输入）。三落点：
  ① **L0→L1 前置保险级联**（Jev-Verified Cascade 模式）：noul「此段含值得长期记住的
  用户事实吗？」P<0.3 跳过 chat 抽取——便宜哨兵挡在贵模型前，比 chat 模型当保险
  便宜一个量级且有概率阈值；
  ② **arbitrate choice 化**：每候选 choice{new,duplicate,contradicts}，替换
  prompt-and-parse；低置信进待审（顺治 T002 相关的漏判兜底问题）；
  ③ **consolidate noul 化**：近重复「语义等价吗」判定。
  凭证：`openrouter/engram`（credentials 域，2026-10-03 入库）。
  **前置条件**：中文效果评测先行（官方案例全英文，engram 全中文记忆，p(yes) 校准度
  未验证）；**风险**：/api/alpha/decisions 为 alpha 面、32k 上下文、arbitrate 的
  target_id 指认是开放集合（choice 只判类型 + top1 相似当 target，或动态构造 criteria
  编号选项——设计期定）。不适配：extract 开放抽取 / persona 生成（保持 chat 模型）。

## Deferred / 交叉引用

- prompt_version 归因列（atoms/scenarios）→ **P001 Deferred 已挂**，不重复开
- 实体归并 LIMIT 1 加 ORDER BY → P001 Deferred「杂项」已挂（本线 T005 补全另两半）
- 结构观察（非缺陷，登记备查）：敏感标记在 L2 丢失——scenarios 表无 sensitive 列，
  敏感原子进场景后场景层面无标记（决策 001「纯标记」口径下无害）
