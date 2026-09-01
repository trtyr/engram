# Roadmap — circle

## Done

- [x] **C-0 四维分离审计**（2026-09-01）：功能/前端/缺口/API 全盘摸清，审计报告落 `docs/design/circle-audit.md`，P1/P2/P3 分级清单成文。

## Next（实施轮待办源，用户过目后另开 goal）

### P1（便宜、直接修，实施轮优先）

- [ ] **详情补「相关实体」**：get_entity 加共现邻居查询 + 前端详情「相关实体」chips（消除详情/图谱割裂）
- [ ] **实体级去重/合并建议**：同名同义实体（「周杰伦」vs「Jay Chou」）提示候选 merge
- [ ] **图可读性：zoom/pan reset 控件**
- [ ] **图可访问性**：canvas 加 aria-label + 键盘焦点替代（节点遍历）

### P2（能力补齐）

- [ ] **圈子语义检索**：后端独立 `GET /memory/entities/search?q=`（复用 search_entities）+ 前端语义搜索框
- [ ] **图可读性：社区聚类**（louvain 或按 kind 分区）
- [ ] **低密度信息呈现**：数据成熟前「列表为主、图为辅」的降级态
- [ ] **实体摘要补生成**：consolidate 门槛 `atom_count>=3` 降或按需单实体触发
- [ ] **实体历史/版本**：摘要版本链 + 历史抽屉，复用 atom revisions 模式

### P3（大项，需拍板——见 open-questions.md）

- [ ] **实体关系升级**（共现→类型化关系，见 topics/relation-model.md）
- [ ] **时间轴视图**（实体时间线 / 全局记忆时间轴）
- [ ] **批量操作**（批量 merge/delete/归档，破坏性需 erase 分权）
- [ ] **实体导出**（圈子独立 csv/json 导出）

## Deferred

- [ ] 无（本轮纯审计，实施留待过目清单后）
