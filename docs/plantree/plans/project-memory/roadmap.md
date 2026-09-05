# Roadmap

## Done

- **概念对齐**（2026-09-04 第一轮）：项目定义（跨会话工作单元，线 vs 点）、第五域定位、两类型先行、plan-tree 整体迁入——decisions 0001~0003。
- **开发项目详情设计对齐**（2026-09-04 第二轮）：plan-tree 概念消融为「规划」分类（0004）；三表模型 + 类型=分类模板 + 分类可增删 + md 轻结构 + 云端为主 + 双入口 + 登记制 + 列表多选管理（0005）。
- **第一版落地**（2026-09-04，goal mtmmgwuu-d6g4rc + 独立 auditor 批准）：0026 迁移三表 + 类型模板（代码常量）+ 15 endpoint API（project scope）+ Web 列表页（CRUD/多选/类型筛选）+ 详情页（Wiki 式左树右内容树状图）。审计发现的路由分流缺陷与截图失效已修复（464be10）。
- **项目记忆 MCP**（2026-09-05）：15 个 project_* 工具并入 /mcp（scope 分权 + tools/list 按 key 过滤 + 名字寻址 + 补丁式更新）+ 精确寻址读（索引模式 / project_doc_search 行级搜索 / project_doc_get 区间精读，零截断）+ 审查修复（改名撞名 409、空名 400、跨项目寻址 404 归属校验、孤儿分类治理下沉 service）。验证：cargo 201 + vitest 45 全绿 + 真机 JSON-RPC E2E。
- **AI 入口定板**（2026-09-05，0006）：AI 入口 = MCP 工具面（memory.py 子命令冻结为备用）；检测本地导入 = AI 文件工具 + MCP（约定非代码）；context pack 协议降级为 instructions 约定，LLM 项目简报进 Deferred。

## In Progress

（无——第一版已交付，等排期继续下一批）

## Next

1. **knowledge 检索打通**：project_docs 是否接 wiki 全文检索管线（open-questions 1）。
2. **调研类型分类用户确认**：六分类草案已实现，正式定稿需用户确认（open-questions 已决节挂账）。

## Deferred

- 自定义类型扩展位的具体机制（等两类型跑通）。
- plan-tree skill 退役/薄壳化时机。
- 轻结构字段集细化（frontmatter jsonb 已预留，状态标签等具体字段待定）。
- LLM 蒸馏「项目简报」（近期文档 → 短摘要，走 LLM 任务队列）——大项目文档规模成为真实痛点再议（0006）。
