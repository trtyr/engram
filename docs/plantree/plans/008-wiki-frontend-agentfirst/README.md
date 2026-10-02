# P008 · wiki 前端 agent-first 重设计

## 范围
web 前端 Wiki.tsx 及相关组件重设计，对齐 P004 后的 agent-first wiki 架构（harness 12 工具维护+MCP 28 actions），让 agent 干活可见、结果可直接查看、运维面板补全。

## 权威
- 想法源：ideas/inbox.md「wiki 前端 agent-first 重设计」（已晋升）
- 拍板①（2026-10-02）：wiki 前端对齐 agent-first
- 拍板②（2026-10-02）：**人审机制整体移除**——不是「前端不升级」保留现状，而是系统里人审相关的东西全删（后端 actions/端点/service+前端面板+测试）；agent 产出直接生效，不存在审批环节
- 受影响模块：web（Wiki.tsx）、server/wiki-engine（review 模块）、server/mcp、server/api

## 侦察结论（2026-10-02，删除边界已确认）
- **harness/core 零依赖**：wiki_agent.rs 与 wiki_docs/ 无任何 review/proposal 引用——移除不伤 agent 写路径（安全）
- **写入点**：INSERT 语句实际在 review.rs:63/95（create_items 与 create_lint_items 的实现）+ingest.rs（织入退役链的 LLM flag 段+purpose_suggestion 段——两段都写 wiki_review_items）；lint_deep.rs:151 是调用点（调 create_lint_items）——2026-10-02 审查修正
- **读取/处理**：repair_ops.rs reviews/review_resolve 两 fn+service.rs apply_proposal（put_page 包装）
- **表**：wiki_review_items（0012 建）——迁移只增不改纪律 → **留表停写**，代码全删
- **review_item_ids 字段零消费**：lint_deep 删该 report 字段后无任何前端/ crate 消费（已 grep 验证），删除安全——2026-10-02 审查确认
- **前端**：Wiki.tsx InboxPane（inbox 面板）+ReviewAndProposals（ops proposals tab）
- **测试**：wiki_test 3 用例（human_page_produces_proposal_not_overwrite/review_resolve_miss_returns_not_found/lint_deep_writes_review_items）+mcp_test action 计数 28→26+golden 重生成

## 文件
- roadmap.md：任务拆分与状态

## 文件
- roadmap.md：任务拆分与状态
