# Roadmap

## Done

- **概念对齐**（2026-09-04 第一轮）：项目定义（跨会话工作单元，线 vs 点）、第五域定位、两类型先行、plan-tree 整体迁入——decisions 0001~0003。
- **开发项目详情设计对齐**（2026-09-04 第二轮）：plan-tree 概念消融为「规划」分类（0004）；三表模型 + 类型=分类模板 + 分类可增删 + md 轻结构 + 云端为主 + 双入口 + 登记制 + 列表多选管理（0005）。

## In Progress

（无——等设计细节排期）

## Next

1. **数据模型与迁移设计**：三表 DDL（projects / project_locations / project_docs）、类型预设分类、轻结构字段集（open-questions 1/2）。
2. **API 设计**：项目 CRUD（含多选批量删除）、文档 CRUD、分类管理、context pack 接口。
3. **AI 侧接入**：memory.py 项目子命令（建项目/改文档/检测本地导入），扫描约定与增量策略（open-questions 3）。
4. **plan-tree 导入器**：docs/plantree/ markdown → 「规划」分类下文档（0004 语义）。
5. **Web 页面**：项目列表页（类型筛选/多选/CRUD）+ 详情页（Wiki 式左树右内容）。
6. **调研类型骨架确认**：六分类预设用户确认后定稿。

## Deferred

- 自定义类型扩展位的具体机制（等两类型跑通）。
- plan-tree skill 退役/薄壳化时机。
- 本根自身迁入项目域「规划」分类（狗粮闭环，等能力覆盖）。
