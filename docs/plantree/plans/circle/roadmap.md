# Roadmap — circle

## Done

- [x] **C-0 四维分离审计**（2026-09-01）：功能/前端/缺口/API 全盘摸清，审计报告落 `docs/design/circle-audit.md`，P1/P2/P3 分级清单成文。

### 实施轮（2026-09-01，goal mtiojzx1-d58lok）——13 项全量落地

**P1（4 项）**

- [x] 详情补「相关实体」（get_entity neighbors + 前端相关实体 chips）
- [x] 实体级去重/合并建议（name 归一化疑似重复提示）
- [x] 图 zoom/pan reset 控件
- [x] 图可访问性（canvas aria-label + 键盘焦点）

**P2（5 项）**

- [x] 圈子语义检索（GET /memory/entities/search + 前端 debounce 语义搜索）
- [x] 图社区聚类（按 kind 扇区初始布局，力导向后同类聚拢）
- [x] 低密度信息呈现（稀疏态诚实提示）
- [x] 实体摘要补生成（consolidate 门槛 ≥3→≥1）
- [x] 实体历史/版本（entity_revisions + 历史抽屉）

**P3（4 项）**

- [x] 实体关系升级 A 路线（entity_relations 表 + 蒸馏抽取 + 有向图边 + 关系 CRUD）
- [x] 全局记忆时间轴（GET /memory/timeline + 图谱/时间轴切换）
- [x] 批量操作（POST /memory/entities/batch，erase scope + confirm 短语）
- [x] 实体导出（GET /memory/entities/export）

## Deferred

- 无（13 项全部落地；数据密度自然积累，社区聚类可后续换 louvain）
