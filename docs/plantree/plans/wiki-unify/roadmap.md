# Roadmap

## Done

- **深度审计**（2026-09-02，goal mtk5yz4l）：[docs/design/wiki-audit.md](../../../design/wiki-audit.md)——后端满配真实工作，缺口全在前端体验层 + 自动织接线
- **后端合一**（bb98d07）：knowledge_api 端点并入 /wiki 前缀（`/knowledge/*` 保留兼容别名，不进 OpenAPI）
- **自动织接线**（c37ede3）：上传文档 ready 后自动触发 `/wiki/ingest {document_id}`（两步思维链织入）
- **前端融合**（a69fcb3）：一个 Wiki 页（tabs：文档/页面/图谱/洞察/Lint/提案/源数据/目的），删 Knowledge.tsx（改 DocumentsPane 具名导出）+ 侧栏「知识库」项
- **图谱体验升级**（c1b5804）：hover 高亮邻居 / 拖拽（captor-disable）/ 缩放三控件 / 边按 weight 编码 / 位置缓存（localStorage）——Playwright 实测 9 节点拖拽缓存写入
- **Obsidian 目录树 IA**（2026-09-03，goal mtkurxdv）：Wiki 前端重做成目录树（folder 层级，迁移 0025 加 folder 字段 + 蒸馏按 page_type 归文件夹）+ Markdown 阅读 + 图谱独立视图（树/图切换）；运维 5 项（洞察/Lint/提案/原料/目标）收二级入口，文档收收件箱——消除 8 平铺 tab

## Next

（合并方案四项全部落地，无剩余；Deferred 项见下）

## Deferred

- 多格式文档（pptx/xlsx/图片）——P3
- 多模态图片摄入（视觉模型）——P3，成本高
- 低密度图谱降级提示——P3
- Obsidian 插件体系 / 全量编辑器 / 实时协作 / 多设备同步（超范围）
