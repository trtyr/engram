# P008 roadmap

## Done

**P008 全部落地（2026-10-02）：**

- [x] **T001 · 人审机制移除（后端）** ✓ 8b97b26——review.rs 模块/3 service fn/lint_deep+ingest 段/MCP 2 action/HTTP 4 端点全删；repair_ops.rs 改名；表 wiki_review_items 留停写；golden 28→26
- [x] **T002 · 人审移除（前端+测试）** ✓ 8b97b26——InboxPane/ReviewAndProposals/ProposalsPane 删+panel 收敛；wiki_test 4 用例（含 plan 漏记的 stale_annotation）+auth_test wiki_proposals 用例删；mcp_test:762 断言修正
- [x] **T003 · Agent 维护流面板** ✓ f118992——HTTP POST /wiki/ingest（harness 语义重建）+AgentPanel（下发/轮询/报告卡/历史表）
- [x] **T004 · 版本管理 UI** ✓ 7522406——HTTP 3 端点（versions 列表/内容/restore）+VersionsPanel（时间线/对比/回滚）
- [x] **T005 · 体检补全** ✓ b27d235——GapsPanel（query-gaps）+DuplicatesPanel（duplicates+merge 确认）两 tab
- [x] **T006 · 语义修正** ✓ 5ad386b——「织入/摄取/收件箱」退役词全 UI 清零
- [x] **T007 · todos 前端**（ideas 晋升并入）✓ 063d34b——TicketDetail 主从模式+WikiMarkdown 渲染 body

## In Progress

（空）

## Next

（空——P008 七任务全部落地，见 Done；二期需求另立 plan）

## Deferred

- DocumentsPane 原文库增强（search/chunks 浏览）——独立评估
- harness 预算/档位前端配置——随生产部署后真实需求
