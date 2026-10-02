# P007 Roadmap

> 任务唯一权威。模型定稿见 README。

## Done

- [x] **T001 · 迁移 0065**（446f627）：study_tracks+study_track_items 两表+track-position 索引；断言 65+两表并入全表存在性测试。
- [x] **T002 · storage repo**（efd098e）：repo/study.rs CRUD+learned_at 状态机语义（learned 记时/离开清空）+track_progress；单测 3/3。
- [x] **T003 · core StudyService**（ec3761b）：topic CRUD/item_add 尾部追加/unit_set 三态校验/topic_get 全量【track+items+progress+next_up 派生+in_progress】；单测 4/4（核心契约绿）。
- [x] **T004 · MCP study 工具面**（3bae446）：study 单入口 8 action；SCOPES 12→13；dispatch 三表同步；golden 重生成；集成测试 2/2（生命周期含 get 核心契约+:ro 拒写）；域工具 11→12 计数同步。
- [x] **T005 · 蒸馏判据同步**（e08c593）：L1 收录判据加学习进度排除（进度归 study；学习背景与偏好照收）；distill_test 31 无破坏。
- [x] **T006 · 工作流手册**：《study×wiki×harness×memory 学习工作流分工手册》落档 engram projects（01a0fba1-506b，doc_search 18 行命中）。
- [x] **Demo · 真实 track 留档**：study_mcp_lifecycle 以「RAG 入门」语义全链跑通（add→3 items→unit_set learned→get 全量留档输出：进度 1/3+next_up 2+资料清单）。

## Done

**二期（做全）六任务全落地（2026-10-02）：**

- [x] **T007 · HTTP 端点** ✓ 433b589——study_routes 8 端点+utoipa paths 注册+openapi 快照同步+:ro 读放行
- [x] **T008 · HTTP 集成测试** ✓ 87c3280——CRUD 全流程+:ro 拒写+错 scope 403（study_api_test 4 用例）
- [x] **T009 · 前端学习页** ✓ a7718c0——Study.tsx（track 卡片+进度条+三态勾选+[[wiki]] 互链+表单）+App 注册；build/tsc/lint 净
- [x] **T010 · harness 联动** ✓ eb51025——工具 10→12（study_list/study_update_item）+prompt 学习协同纪律+集成测试（DB learned 断言）
- [x] **T011 · SRS 复习机制** ✓ 169d7ad——0066 加 needs_review/review_due_at+item_set_review/reviews_due（NULL=立即到期）+MCP/HTTP 双面+前端到期徽标
- [x] **T012 · journal 时间线** ✓ 169d7ad——0066 加 study_track_journal+MCP journal_add/list+HTTP /journal+topic_get 带 recent_journal+前端时间线

二期合计 6 commit（433b589/87c3280/a7718c0/eb51025/169d7ad+plan）；
study actions 8→12（item_set_review/reviews_due/journal_add/journal_list）；
HTTP 面 8→11 端点（+reviews+journal 两路径）；golden 已重生成。

## Next

（空——二期收官）

## Deferred

- 前端学习页（track 全景视图）——MCP-only 一期先跑
- harness 协同（喂资料→建页→顺手勾知识点）
- SRS/needs_review 复习机制
- journal 进度时间线
