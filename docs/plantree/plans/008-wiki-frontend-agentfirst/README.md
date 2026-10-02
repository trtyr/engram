# P008 · wiki 前端 agent-first 重设计

## 范围
web 前端 Wiki.tsx 及相关组件重设计，对齐 P004 后的 agent-first wiki 架构（harness 12 工具维护+MCP 28 actions），让 agent 干活可见、结果可直接查看、运维面板补全。

## 权威
- 想法源：ideas/inbox.md「wiki 前端 agent-first 重设计」（已晋升）
- 拍板（2026-10-02）：**不做人审环节**——agent 产出直接生效，人看结果不审批；reviews/proposals 前端保持现状不升级
- 受影响模块：web（Wiki.tsx/DocumentsPane/新增组件）、server/api（如需 job report 查询端点复用 /jobs）

## 非目标
- 人审中心/reviews+proposals UI 升级（用户明确否决）
- wiki 后端功能变更（纯前端消费现有端点；缺数据才补只读端点）
- 编辑器形态改造（现有 tree+编辑器保留）

## 文件
- roadmap.md：任务拆分与状态
