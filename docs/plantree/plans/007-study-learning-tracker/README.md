# P007 · study 学习路线图跟踪器

## 范围

学习过程的路线图跟踪：领域 track → 知识单元 item（状态机）→ 挂 wiki 页。
**只管「学没学/学到哪/下一步学啥」**；知识内容归 wiki（harness 维护），原文归 documents，感悟叙事归 memory。

来源：EN-33 + 用户实际学习体验（学习助手会话现场痛点）+ ideas/inbox 2026-10-02 想法晋升（用户拍板：独立域）。

## 模型定稿（2026-10-02 与学习助手讨论定案）

- **Topic**: name / goal（学到什么程度算完）/ status(active|paused|done)
- **Unit**: name / status(**not_started|learning|learned**——事实性命名，弃 mastered) / wiki_slugs[] / doc_ids[] / learned_at
- next_up **不实体化**（pending 按 position 排序的派生视图）
- journal 进度日志一期不做（learned_at 兜底，叙事归 memory）
- SRS/掌握度评价层一期不做（真实痛点出现再加 needs_review+due_at 起步）
- 单库：无 library_id（wiki 单库终局）

## Affected Modules

server/storage（迁移 0065+repo）、server/core（StudyService）、server/mcp（study 工具面）、server/distill（判据同步）、web（学习页，二期）。

## 文件

- roadmap.md —— 任务唯一权威
- open-questions.md —— 悬而未决
