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
| P008 | [wiki-frontend-agentfirst](plans/008-wiki-frontend-agentfirst/README.md) | web, server/wiki-engine, server/mcp, server/api | planning | wiki agent-first 对齐+**人审机制整体移除**（拍板②：后端 actions/端点/service+前端面板+测试全删，表留停写）；Agent 维护流面板/版本 UI/体检补全/语义修正 | — | T001 人审移除（后端） |

## Ideas

- [ideas/inbox.md](ideas/inbox.md) — 低承诺想法池

## 维护纪律

- Plan ID 项目级递增（P001…），永不复用；任务 ID Plan 内 T001…
- 决策进 `plans/<NNN>/decisions/`，待拍板进 `open-questions.md`（只留未决）
- 定稿的架构级结论沉淀 engram 档案（doc_add 决策类），本地留链接
- 文档清单变化时同步仓库根 AGENTS.md 索引段
