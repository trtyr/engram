# Roadmap

## Done

- **深度审计**（2026-09-02，goal mtk5yz4l）：[docs/design/wiki-audit.md](../../../design/wiki-audit.md)——后端满配真实工作，缺口全在前端体验层 + 自动织接线

## Next（按优先级）

审计结论：后端（两步摄入/四信号/Louvain/洞察/级联/ingest document_id）已全健在，**合并方案的实施重点 = 前端体验 + 自动织接线**，后端只做「/knowledge 并入 /wiki」。

1. **自动织接线（P1）**：上传文档后自动触发 `/wiki/ingest {document_id}`（后端已支持）+ 前端「织入中」状态
2. **前端融合**：一个 Wiki 页（tabs：文档/页面/图谱/人审/提案/目标），删 Knowledge.tsx
3. **图谱体验升级（P1+P2）**：hover 高亮邻居 / 拖拽 / 缩放控件 / 边按 weight 编码 / 位置缓存
4. **后端合一**：knowledge_api 端点并入 /wiki 前缀（+ /knowledge 兼容别名）

## Deferred

- 多格式文档（pptx/xlsx/图片）——P3
- 多模态图片摄入（视觉模型）——P3，成本高
- 低密度图谱降级提示——P3
- Obsidian 插件体系 / 全量编辑器 / 实时协作 / 多设备同步（超范围）
