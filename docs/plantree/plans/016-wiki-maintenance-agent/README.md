# P016 · maintain_wiki Agent（wiki 定期维护）

> 2026-10-05 立项即收官（单日交付）。动机：wiki 的 lint/repair/duplicates/insights 全是人触发按钮，
> 没人点就永不体检；前端运维 tab 暴露运维操作给用户（拍板：不该开放）。

## 形态（对齐 maintain_memory 模式）

- 节律每天巡逻：rhythm_maintain_wiki 自续明日桶（rhythm-maintain-wiki-YYYYMMDD 幂等）+ bootstrap 启动补建
- 巡逻编排（wiki-engine/src/maintain.rs）：lint 全量 → repair 确定性修复自动执行（三级边界现成：
  自动做死链改写/去链/stub/孤页回挂；留痕做同标题合并；语义级不做）+ 向量回填 → duplicate_candidates
  只报告 → lint_deep LLM 深检入队（有 issue 才触发）→ WikiPatrolReport 落 jobs progress
- 单飞守卫：WORKFLOW_KINDS 收纳 maintain_wiki（domain=wiki）
- API：GET /wiki/patrol/latest（最近报告+下次巡逻时间）、POST /wiki/patrol（手动触发，单飞守卫）
- 前端：Wiki 运维 tab 七面板下架（Gaps/Duplicates/Agent/Insights/Lint/Sources/Purpose），
  换只读 PatrolPane 巡检报告视图（520 行运维代码退役）

## Done

- 2026-10-05 全量落地：maintain.rs + handler 注册 + 节律 + API + 前端 PatrolPane + 巡逻测试 2/2
  （空库零问题 + 死链修复后 lint 递减+深检入队）
- 门禁：fmt 0 / clippy 五 crate 0 / wiki-engine+jobs 测试全绿 / web 四件套全绿

## 形态决策记录（2026-10-05 审计后补）

- 初版实现走纯确定性编排（无 LLM 循环）；审计指出 objective ③ 要求复用 Harness 工具循环，
  用户拍板「照③硬做」→ 补 patrol_agent.rs（JSON 协议工具循环：list_pages/get_page/finish，
  12 步预算闸门，maintenance system prompt，Purpose::WikiLint 复用），
  巡逻本体保持确定性（lint→repair→回填→duplicates），Agent 接在其后做语义裁决+维护纪要
  （agent_summary/manual_actions 进报告；Agent 失败降级 warn 不阻塞巡逻）。
- 顺手修生产 bug：rhythm_maintain_wiki handler 未注册（首部署日志里该任务 failed=未注册任务种类）——
  register_patrol_rhythm 并入 ingest::register_handlers 收口。
