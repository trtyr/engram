# Roadmap

## Done

（规划中，尚无落地）

## Next（按优先级，等 open-questions 拍板后实施）

1. **后端合一**：knowledge_api 端点并入 /wiki 前缀（+ /knowledge 兼容别名），统一成一个 Wiki 域
2. **自动织管线**：上传文档 → 存原料（documents/chunks）→ 后台队列自动织页面/链接（sha256 去重 + 失败隔离 + jobs 可观察）
3. **前端融合**：一个 Wiki 页（tabs：文档/页面/图谱/人审/提案/目标），删 Knowledge.tsx
4. **知识图谱升级**：Obsidian graph view——hover 高亮邻居 / label 常显分级 / 拖拽 / 缩放 / 点击跳转 / 类型图例

## Deferred

- Obsidian 插件体系 / 全量编辑器（超范围）
- 实时协作 / 多设备同步（超范围）
