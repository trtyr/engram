# Open Questions

第一版落地（2026-09-04，goal mtmmgwuu-d6g4rc）后的剩余未决：

1. **文档与 wiki 域的关系**：project_docs 自带 markdown 内容已定，但要不要打通 wiki 的检索管线（文档可被全文检索）？还是项目文档检索走独立的 LIKE/FTS？
2. **轻结构字段集**：frontmatter jsonb 已预留，但元数据具体有哪些（状态标签/负责人/顺序/置顶）？状态枚举值？（字段集细化在 Deferred）

## 已决（2026-09-05，MCP 落地后）

- **AI 检测导入的形态**（原 3）：不做平台功能、不做扫描代码——AI 用自身文件工具读本地 markdown → MCP project_doc_add 写入；目录与分类映射是约定文字（instructions），memory.py 项目子命令冻结为备用入口。见 [0006](decisions/0006-mcp-entry-and-context-pack-downgrade.md)。
- **context pack 的接口形态**（原 4）：不设专用端点/协议——开工 = 索引 → 搜索定位 → 区间精读（按需自组，比固定打包更省），收工 = doc_add/doc_update + project_update 改状态（落点约定写进 instructions）。LLM「项目简报」进 Deferred。见 [0006](decisions/0006-mcp-entry-and-context-pack-downgrade.md)。

## 已决（随第一版落地）

- **多主机绑定**（原 4）：同一 repo 多主机 = 多行 `project_locations`（host/path/purpose 区分用途），非多个 project。
- **删除语义**（原 5）：项目删除 = FK ON DELETE CASCADE 级联硬删 locations/docs，无软删（project 域暂不落审计行，与 wiki source-cascade 的审计哲学不同）。
- **调研分类**（原 7，定稿仍待用户确认）：六分类「待查/线索/资料/结论/疑点/证伪」已实现为代码常量草案，正式定稿待用户确认。

更早的已决（挂靠方式/骨架表达/plan-tree 形态/录入入口）见 [decisions/](decisions/)。
