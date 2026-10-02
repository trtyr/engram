# P008 roadmap

## Done

（空）

## In Progress

（空）

## Next

- [ ] **T001 · Agent 维护流面板**：ops 新增「维护 Agent」tab——下发区（URL/文本→喂给维护 Agent，POST /wiki/ingest 返 job_id）+**内联 job 进度**（轮询 GET /jobs/{id}+events，展示轮数/工具调用/预算）+agent report 卡片（建/改页列表可点击直达、degraded 标记、耗时）+历史 run 列表。
- [ ] **T002 · 版本管理 UI**：页详情加「历史」入口——GET versions 列表（时间线）+两版本内容对比（textarea diff 或简单双栏）+restore_version 回滚按钮（破坏性确认）。
- [ ] **T003 · 体检补全**：ops 补 query-gaps 面板（GET /wiki/query-gaps——知识缺口列表+「喂原料」快捷入口）+duplicates 面板（GET /wiki/duplicates——重复页对+跳转 merge 确认）。
- [ ] **T004 · 语义修正+ops 重排**：全 UI「摄取/ingest」文案改「喂给维护 Agent」；ops tab 重排（维护 Agent/体检/版本/来源/purpose——proposals tab 移除入口保持后端不动）；收尾回归。

## Deferred

- DocumentsPane 原文库增强（search/chunks 浏览）——独立评估
- harness 预算/档位前端配置——随生产部署后真实需求
