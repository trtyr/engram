# P007 Roadmap

> 任务唯一权威。模型定稿见 README。

## Done

（空——未开工）

## In Progress

（空）

## Next

- [ ] **T001 · 迁移 0065**：study_tracks（id/name/goal/status/created/updated）+ study_track_items（id/track_id/name/status/position/wiki_slugs jsonb/doc_ids jsonb/learned_at）；migrations_test 断言 65。
- [ ] **T002 · storage repo**：study_tracks/items CRUD（repo/study.rs）。
- [ ] **T003 · core StudyService**：topic CRUD / unit 状态机（not_started→learning→learned）/ next_up 派生 / topic_get 全量（进度+下一步+资料清单一次拿全——跨会话恢复学习上下文的核心查询）。
- [ ] **T004 · MCP study 工具面**：study add/get/list/unit_set/item_add/topic_update 归档；dispatch 读写分类+golden 快照+mcp_test 计数+help 文档全流程（照 T007 先例）。
- [ ] **T005 · 蒸馏判据同步**：进度类原子不再进 memory（蒸馏 prompt/判据面更新）——P003 教训：判据面不同步旧路径继续产过时原子。
- [ ] **T006 · 工作流手册**：《study×wiki×harness 分工》（study 管过程状态/wiki 管知识/documents 管原文/memory 管人物层事实）+ memory「正在学 X」原子退役口径。

## Deferred

- 前端学习页（track 全景视图）——MCP-only 一期先跑
- harness 协同（喂资料→建页→顺手勾知识点）
- SRS/needs_review 复习机制
- journal 进度时间线
