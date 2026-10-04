# P012 Roadmap

> 任务身份/状态/顺序的唯一权威。Planning——开工顺序 T001→T002→T003→T004→T005。

## Done

- **T001-T003** ✅ 97e87d7（2026-10-03 夜间冲刺，goal musad761-gko7jm）——六工具 JSON 协议循环
  + retired_at 软删（迁移 0069）+ organize::run settings 开关分派（默认旧路径）；
  p012 两测（循环组织/上限硬顶）全绿

## Next

- [x] **T001 · 工具集实现**
  六件 + finish（见 README 关键设计 1）：`atoms_pending` / `scenarios_search`（tsv+向量
  双路）/ `scenario_get` / `scenario_write`（create/update 合一，成员增删）/ 
  `scenario_merge`（from 软删成员迁入）/ `scenario_retire`（软删释放原子）。
  全部走 repo 层复用现有 SQL；写操作落审计事件。验证：单测（repo 已有基础）+ 工具
  契约测试。
- [x] **T002 · agentic loop runner + 成本闸（联动 P011 T007）**
  多轮 tool-use 循环：max_steps 硬顶（20）+ 每任务 token/cost 上限 + 超时；JEV 式
  降级不可用（本任务无降级路径，模型不可用 = 任务失败可重试）。**P011 T007 的
  budget_tokens 熔断在本任务一并落地**（runner 层统一闸）。
- [x] **T003 · organize handler 接入**
  主组织流程替换为 agentic loop；`converge_only` 快速通道与快照收敛保持确定性代码
  不动（scenario_converge.rs 原样）；空散落原子时的兜底投递语义保留。链式入队
  persona 语义保留（touched 场景 + removed_texts 由工具写操作推导）。
- [x] **T004 · 烟测 + 对齐评测（评测执行中）** ——✅ 框架+基线（见 evaluation.md）；
  agentic 侧受免费模型限流数据不足，付费复测指引已备，切换策略留用户拍板
  ① provider tool-use 烟测（现有模型能否稳定走完工具循环）；② 对齐评测：同一批
  真实原子，旧 organize vs agentic organize 产出对比（场景数/重复率/成员归属合理性/
  成本时长）；③ 验收线用户拍板。
- [ ] **T005 · 门禁 + 切换**
  workspace/clippy/web 门禁；生产切换策略（默认开 or 设置开关灰度——拍板）。

## In Progress

（空）

## In Progress

- [ ] **T005 · 门禁 + 切换（拍板已更新 2026-10-03 深夜）**
  **用户拍板：不需要旧版兼容**——旧单发路径整体删除，agentic 成为唯一实现：
  - 删 organize.rs 旧单发组织段（fetch_scenarios/build_organize_prompt/
    organize_with_llm/apply_actions/Action 解析）+ organize_agentic_flag 开关 +
    settings.organize_agentic
  - 旧 prompt v2 退役（系统提示词已在 organize_agentic::system_prompt）
  - T004 评测报告的「切换策略」结论作废——不复测对比再拍板，直接切
  - 部署即 agentic（生产首次部署就是新形态，无灰度）
  - **已执行 ✅ 8f11d94**（同夜）：organize.rs 重写（503→169 行，-389 行旧链）、
    prompts v2 退役、DistillParams.step 避让、测试面全量对齐 agentic mock；
    顺带修 agentic 化引入的 scenario_write 重算缺 bind 真 bug（t003 逮住）。
    distill 39 全绿；clippy 0；fmt 净

## Deferred

- 场景表规模治理（如需）——agentic 形态天然解除 top100 截断，观察后再议

## 交叉引用

- P011 T007（LLM 熔断）→ 本线 T002 一并落地
- P011 T003（场景成员双轨漂移）→ scenario_write 的成员增删是收敛双轨的时机，
  实现时评估一并修（scenario_id 与 atom_refs 单点同步）
