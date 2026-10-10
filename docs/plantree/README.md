# engram · Plan Tree

规划工作面（本地）。**分工约定（2026-09-28）**：Mia 负责把问题整理成决策点并执行；trtyr 只负责拍板。

## 权威顺序

1. **代码与运行行为**（真源）
2. **engram projects 档案 `name=engram`** —— 架构事实 / 风险详情 / 决策定稿（索引见仓库根 AGENTS.md）
3. **本树** —— 活跃规划状态 / 整改追踪 / 待拍板问题

本树只做规划状态与决策流转，**不复制档案内容**（链接不复制，防双权威漂移）。

## Baseline（项目上下文）

- [baseline/README.md](baseline/README.md) — 入口与档案链接
- [baseline/module-map.md](baseline/module-map.md) — 模块键（Plan 的 Affected Modules 用）
- [baseline/risk-hotspots.md](baseline/risk-hotspots.md) — 风险热点速览
- [baseline/test-and-release-gates.md](baseline/test-and-release-gates.md) — 门禁与红线
- runtime-flows / storage-and-state：不建本地副本，直读档案（运行时视图 `01a0e6f4-59c6` · 数据层 `01a0e6f3-d2cb`）

## Active Plans

| ID | Plan | Affected Modules | Status | Current Phase | Last Landed | Next Target |
|---|---|---|---|---|---|---|
| P001 | [risk-debt-remediation](plans/001-risk-debt-remediation/README.md) | scripts/backup.sh, deploy/.env.example, README.md, distill, core/memory | executing | T001-T004 Done 并已提交（9be3bec..bde6092），T005 回填等实例确认 | 2026-10-01 全量提交 9be3bec..bde6092（门禁 bg_ala7qpal 全绿） | T005 存量回填（Q004） |
| P002 | [mcp-surface-fixes](plans/002-mcp-surface-fixes/README.md) | server/mcp（外部组件不归本线） | done | T001-T005 全 Done（EN-10/11/12/24/25 全 resolved） | 2026-10-01 95a115d + cargo test --workspace EXIT=0（73 target ok） | —（生产重部署后 B 修复生效；Deferred 见 roadmap） |
| P003 | [review-findings-remediation](plans/003-review-findings-remediation/README.md) | server/distill, server/core, server/mcp, scripts, docs-repo | done | T001-T003 全 Done（P1-1 永动机修复 + Consider×7 + Nit×4 + clippy 存量清零） | 2026-10-01 dd23392/17e5275/72fac41 + workspace EXIT=0（73 target ok） | —（重部署 checklist 见 roadmap Deferred） |
| P004 | [wiki-document-ingest](plans/004-wiki-document-ingest/README.md) | server/core/wiki_docs, server/mcp, web, server/parsing | done | 夜间长任务收官（2026-10-02）：T001-T010 全落地（harness 主体+织入退役+三篇救活+EN-31/32 关闭）；真实 demo 已跑通（LangChain 链接→抓取→建 3 页互链 6 条，118s） | ae069d0/2cd036a/aaae960/2c22833/ca49dc6/15ae465 | 手册=projects《wiki 维护工作流手册》 |
| P005 | [logging](plans/005-logging/README.md) | server/api, server/core, server/llm, server/storage, server/jobs, web | done | 六任务全落地（2026-10-02）：logs 表+PgLogLayer+request-id 贯穿+LLM 调用日志+6 审计点+GET /logs+前端日志页+覆盖面 sweep | 9b99c55/cd51c9f/a6f1719/2755a0d/e1c5da4/0c64198 | 保留期 info30d/debug7d |
| P006 | [error-handling](plans/006-error-handling/README.md) | server/core, server/storage, server/llm, server/jobs, server/mcp, server/api, server/distill, web | done | 五任务全落地（2026-10-02）：EngramError+17 码注册表+wiki_docs 桥+internal_bug 告警升级+边界 category/request_id+错误码全表；传播链剩余域 Deferred | d5d7d12/619662c/91daebc/b387089 | 渐进下沉（Q001） |
| P007 | [study-learning-tracker](plans/007-study-learning-tracker/README.md) | server/storage, server/core, server/mcp, server/api, server/distill, web | done | 一期 MCP 域（446f627..e08c593）+二期做全（433b589..169d7ad）：HTTP 11 端点/集成测试/前端学习页/harness 联动（工具 12）/SRS/journal——四层全通 | 433b589/87c3280/a7718c0/eb51025/169d7ad | 0066 迁移；actions 8→12 |
| P008 | [wiki-frontend-agentfirst](plans/008-wiki-frontend-agentfirst/README.md) | web, server/wiki-engine, server/mcp, server/api | done | 人审机制整体移除（T001-T002）+Agent 维护流面板/版本 UI/体检补全/语义修正（T003-T006）+todos 详情/markdown（ideas 晋升 T007） | 8b97b26/f118992/7522406/b27d235/5ad386b/063d34b | 表 wiki_review_items 留停写；wiki actions 28→26 |
| P009 | [frontend-ia-observability](plans/009-frontend-ia-observability/README.md) | web, deploy, server/api | done | 任务并入日志并移除任务入口/学习归资产域/日志 7 天窗口+长内容收起/已完成计数/账号拆三标签/网页读取归 AI 功能/生产 codegraph 数据根修复 | 2c5423a/72003f6/dbef672/cb7bf7d | — |
| P010 | [logs-unify](plans/010-logs-unify/README.md) | server/logs, mcp, web, db | done | 日志统一为系统唯一时间线（job_events 并入 logs，迁移 67）；MCP logs 域；前端日志页重建为单线；任务概念退役 | 31ad8a4/2ad412d/891dbff/e2cb949 | — |
| P011 | [memory-audit-debts](plans/011-memory-audit-debts/README.md) | server/distill, server/core/memory, server/storage, server/mcp, web, server/api | executing | 十任务 goal 收官（2026-10-04）：T001-T014/T016-T021 全落地推送 + 生产重部署 + 全量重蒸；T015（arbitrate choice 化+consolidate noul）按设计后置 | 8f11d94/43cfb21 + 生产 43cfb21 部署 | T015 阶段二（Deferred）；遗留拍板进 P013 |
| P012 | [organize-agentic](plans/012-organize-agentic/README.md) | server/distill, server/storage, server/llm | done | agentic 唯一化收官（2026-10-04）：六工具循环+软删+迁移 0069；旧单发全链删除（8f11d94，-389 行，无开关无兼容）；生产部署+重蒸验证 | 8f11d94（distill 39 全绿/clippy 0/fmt 净） | — |
| P013 | [review-chain-rework](plans/013-review-chain-rework/README.md) | server/core/memory, server/api, server/mcp, web, deploy | planning | 待审通道整体移除 + 生产 JEV 决策模型补配（2026-10-04 拍板登记） | — | README 三个拍板点过会 |
| P014 | [table-filter-ui](plans/014-table-filter-ui/README.md) | web | planning | Excel 式列头筛选：DataTable 组件 + 全站列表页迁移（2026-10-04 拍板登记） | — | 三个拍板点过会 |
| P015 | [distill-agent-unify](plans/015-distill-agent-unify/README.md) | server/distill, server/core/memory, server/search, server/storage, web | planning | 蒸馏链全面 Agent 化：维护 Agent 四工具（查/入库/修改/删除）替代向量仲裁 + embedding 退役评估（2026-10-04 拍板登记） | — | README 四个拍板点过会 |
| P016 | [wiki-maintenance-agent](plans/016-wiki-maintenance-agent/README.md) | server/wiki-engine, server/jobs, server/api, web | done | maintain_wiki 巡逻 Agent 全量落地（2026-10-05）：lint→repair→回填→duplicates + patrol_agent 语义裁决；前端运维 tab 换 PatrolPane | fmt 0/clippy 0/巡逻测试 2/2/web 四件套绿 | — |
| P017 | [project-maintenance-agent](plans/017-project-maintenance-agent/README.md) | server/distill, server/api | done | maintain_project Agent 全量落地（2026-10-06）：per-project 单飞+节律 fanout+四工具循环（list_docs/get_doc/note_issue/finish）+Markdown 报告 | distill 19+14+3 全绿/clippy 0 | — |
| P018 | [full-review-remediation](plans/018-full-review-remediation/README.md) | server/jobs, server/core, server/wiki-engine, server/api, server/mcp, server/distill, server/llm, web, scripts, docs | done | 全线收官（2026-10-08）：T001-T010 + Q001-Q008 全 done（余 .env.example 文档面一处待 docs-sync）；七路审查 61 findings 全处置 | 7520dae / 71df416 / c8fbbaf4 / dd4acf9（各批门禁全绿） | — |
| P019 | [code-review-remediation](plans/019-code-review-remediation/README.md) | server/jobs, cg-bridge, distill, wiki-engine, api, mcp, core, storage, web, scripts | executing | M1+M2 收官：T001 五条 P1（02126f8）+ T002 鉴权族七条（study:ro//search 越权/original:ro/死权限变体/tool_scope 兜底/登录爆破计数/action 旁路）全落地+回归测试，门禁全绿 | 02126f8 / 9f705e6 / M2 commit | Q001-Q006 拍板 + M3（T003 前端可用性族） |

## Ideas

- [ideas/inbox.md](ideas/inbox.md) — 低承诺想法池

## 维护纪律

- Plan ID 项目级递增（P001…），永不复用；任务 ID Plan 内 T001…
- 决策进 `plans/<NNN>/decisions/`，待拍板进 `open-questions.md`（只留未决）
- 定稿的架构级结论沉淀 engram 档案（doc_add 决策类），本地留链接
- 文档清单变化时同步仓库根 AGENTS.md 索引段
