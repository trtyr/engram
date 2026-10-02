# P008 roadmap

## Done

（空）

## In Progress

（空）

## Next

- [ ] **T001 · 人审机制移除（后端）**：删 wiki-engine review.rs 模块+lib.rs mod+repair_ops reviews/review_resolve 两 fn+service apply_proposal+lint_deep create_lint_items 段+ingest LLM flag/purpose_suggestion 两段+pages/repair_ops cascade_dismiss 两行；删 MCP reviews/review_resolve（dispatch action_docs+is_write+is_read+组+wiki.rs Params/handler/match 分支）；删 HTTP 4 端点（repair_review.rs apply_proposal/list_proposals/list_reviews/resolve_review+路由 4 行+paths 4 行+openapi 清单 4 路径）。表 wiki_review_items 留停写。
- [ ] **T002 · 人审机制移除（前端+测试）**：删 Wiki.tsx InboxPane+inbox 面板入口（panel 类型收敛 none|ops）+ReviewAndProposals+ops proposals tab；删 wiki_test 3 用例；mcp_test action 计数 28→26（若断言）；UPDATE_GOLDEN=1 重生成 golden；全回归。
- [ ] **T003 · Agent 维护流面板**：ops 新增「维护 Agent」tab——下发区（URL/文本→喂给维护 Agent，POST /wiki/ingest 返 job_id）+**内联 job 进度**（轮询 GET /jobs/{id}+events，展示轮数/工具调用/预算）+agent report 卡片（建/改页列表可点击直达、degraded 标记、耗时）+历史 run 列表。
- [ ] **T004 · 版本管理 UI**：页详情加「历史」入口——GET versions 列表（时间线）+两版本内容对比（textarea diff 或简单双栏）+restore_version 回滚按钮（破坏性确认）。
- [ ] **T005 · 体检补全**：ops 补 query-gaps 面板（GET /wiki/query-gaps——知识缺口列表+「喂原料」快捷入口）+duplicates 面板（GET /wiki/duplicates——重复页对+跳转 merge 确认）。
- [ ] **T006 · 语义修正+ops 重排**：全 UI「摄取/ingest」文案改「喂给维护 Agent」；ops tab 重排（维护 Agent/体检/版本/来源/purpose——proposals tab 移除入口保持后端不动）；收尾回归。

## Deferred

- DocumentsPane 原文库增强（search/chunks 浏览）——独立评估
- harness 预算/档位前端配置——随生产部署后真实需求
