# Open Questions

第一版落地（2026-09-04，goal mtmmgwuu-d6g4rc）后的剩余未决：

1. **文档与 knowledge 域的关系**：project_docs 自带 markdown 内容已定，但要不要打通 knowledge 的检索管线（文档可被全文检索）？还是项目文档检索走独立的 LIKE/FTS？
2. **轻结构字段集**：frontmatter jsonb 已预留，但元数据具体有哪些（状态标签/负责人/顺序/置顶）？状态枚举值？（字段集细化在 Deferred）
3. **AI 检测导入的形态**：memory.py 项目子命令具体形态？扫描哪些约定目录（docs/、server/docs/…）？增量同步还是一次性导入？
4. **context pack 的接口形态**：AI 开工拉上下文——专用端点（一次拉位置+规划+指定分类最新 N 篇）还是客户端多次调用？收工沉淀（文档更新）走 PUT 即可还是有专门协议？

## 已决（随第一版落地）

- **多主机绑定**（原 4）：同一 repo 多主机 = 多行 `project_locations`（host/path/purpose 区分用途），非多个 project。
- **删除语义**（原 5）：项目删除 = FK ON DELETE CASCADE 级联硬删 locations/docs，无软删（project 域暂不落审计行，与 wiki source-cascade 的审计哲学不同）。
- **调研分类**（原 7）：六分类「待查/线索/资料/结论/疑点/证伪」已实现为代码常量草案，正式定稿待用户确认。

更早的已决（挂靠方式/骨架表达/plan-tree 形态/录入入口）见 [decisions/](decisions/)。
