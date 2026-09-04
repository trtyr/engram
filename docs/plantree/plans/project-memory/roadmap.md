# Roadmap

## Done

- **概念对齐**（2026-09-04 第一轮）：项目定义（跨会话工作单元，线 vs 点）、第五域定位、两类型先行、plan-tree 整体迁入——decisions 0001~0003。
- **开发项目详情设计对齐**（2026-09-04 第二轮）：plan-tree 概念消融为「规划」分类（0004）；三表模型 + 类型=分类模板 + 分类可增删 + md 轻结构 + 云端为主 + 双入口 + 登记制 + 列表多选管理（0005）。
- **第一版落地**（2026-09-04，goal mtmmgwuu-d6g4rc + 独立 auditor 批准）：0026 迁移三表 + 类型模板（代码常量）+ 16 endpoint API（project scope）+ Web 列表页（CRUD/多选/类型筛选）+ 详情页（Wiki 式左树右内容树状图）。审计发现的路由分流缺陷与截图失效已修复（464be10）。

## In Progress

（无——第一版已交付，等排期继续下一批）

## Next

1. **AI 侧接入**：memory.py 项目子命令（建项目/改文档/检测本地导入），扫描约定与增量策略（open-questions 3）。
2. **plan-tree 导入器**：docs/plantree/ markdown → 「规划」分类下文档（0004 语义）。
3. **context pack 接口**：AI 开工拉上下文 / 收工沉淀协议（open-questions 6）。
4. **knowledge 检索打通**：project_docs 是否接 knowledge 全文检索管线（open-questions 1）。
5. **调研类型分类用户确认**：六分类草案已实现，实施前需用户确认（open-questions 7）。

## Deferred

- 自定义类型扩展位的具体机制（等两类型跑通）。
- plan-tree skill 退役/薄壳化时机。
- 本根自身迁入项目域「规划」分类（狗粮闭环，等 plan-tree 导入器）。
- 轻结构字段集细化（frontmatter jsonb 已预留，状态标签等具体字段待定）。
