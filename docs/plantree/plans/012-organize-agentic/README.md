# P012 · organize agentic 重构（L1→L2 组织者 Agent 化）

## 范围
把 L1→L2 的组织者从「单次 prompt 塞全部清单」重构为 **agentic tool-use 循环**：
给模型一组场景维护工具（查原子/搜场景/读写场景/合并/软删），由模型自主探索与决策，
替代 top100 截断 + 一次性全塞的现有形态（distill/organize.rs）。

## 权威
- 用户设计指令（2026-10-03）：「把它当成一个 Agent 来设计，给它几个工具：查看原子 /
  查看当前的所有场景（要灵活）/ 编辑场景（要牛逼）/ 删除场景……不需要一次性把东西
  全部放到上下文里，让它自己去调、自己去查，用模型的智力自己去搞。」
- 联动：循环成本闸与 P011 T007（LLM 熔断缺失）合并实现。

## 关键设计决定
1. **工具集六件 + finish**：`atoms_pending`（散落原子分页）· `scenarios_search`（关键词
   搜场景，tsv+向量双路——「灵活」核心）· `scenario_get`（详情含成员全文）·
   `scenario_write`（create/update 合一，成员可增删——「牛逼编辑」）· `scenario_merge`
   （重复场景合并）· `scenario_retire`（软删）· `finish`（交卷）。
2. **删除一律软删**（归档 + 成员释放回未归组）——模型无物理删权限；物理解散仅保留
   converge 确定性路径（活跃成员=0）。
3. **converge 不交给模型**：快照收敛（成员失效重算/解散）保持确定性代码（scenario_converge.rs），
   它是遗忘级联的一部分；Agent 只管「新原子的组织」。
4. **循环预算**：max_steps 硬顶（建议 20）+ 每任务 token/成本闸（与 P011 T007 一并落地）。
5. **全工具调用落事件流**（logs 域）：每步查/改可回放。
6. 模型路由：organize 用途需 tool-use 能力——provider capability 评估（复用 chat 还是
   新增 capability，实现期定）。

## 风险
- 成本×10（多轮循环）——闸门与循环上限是硬前置，不是可选项
- 模型幻觉删改——软删 + 事件流 + 上限三重兜底
- tool-use 模型质量决定成败——实现前先用现有 provider 过一轮工具调用烟测

## 文件
- roadmap.md：任务与状态
