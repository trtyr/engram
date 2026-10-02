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

## In Progress

**二期（做全，2026-10-02 立项）**——一期 MCP-only 的缺口补全：HTTP 面+前端+联动+复习+时间线。

## Next

- [ ] **T007 · HTTP 端点**：study_routes（8 端点：GET/POST /study/topics、GET/PATCH/DELETE /study/topics/{id}、POST /study/topics/{id}/items、PATCH/DELETE /study/items/{id}）；utoipa paths 注册+openapi 快照同步；读端点 :ro 放行（对齐 MCP 语义）。
- [ ] **T008 · HTTP 集成测试**：CRUD 全流程+`:ro` 拒写+非 study scope 403；测试绿是 T007 完成条件之一。
- [ ] **T009 · 前端学习页**：web/src/features/Study.tsx（track 列表卡片+goal/进度条+节点三态勾选+wiki slug 链接+新建表单）+App.tsx 注册（nav+route）；web build+tsc+lint 净。
- [ ] **T010 · harness 联动**：harness system prompt 加学习协同指引（建页时若命中 study track 知识点，提示/执行 unit_set+item_link——工具面 MCP 同源已可调）；集成测试。
- [ ] **T011 · SRS 复习机制（最小）**：0066 迁移（items 加 needs_review 布尔+review_due_at）；MCP unit_set 扩展 needs_review 参数；复习队列查询（review_due_at 升序）；前端复习入口随 T009。
- [ ] **T012 · journal 进度时间线**：0066 同迁移加 study_track_journal（track_id/ts/note）；MCP journal_add/journal_list；topic_get 带最近时间线。

## Deferred

- 前端学习页（track 全景视图）——MCP-only 一期先跑
- harness 协同（喂资料→建页→顺手勾知识点）
- SRS/needs_review 复习机制
- journal 进度时间线
