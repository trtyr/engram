# Tooling Fix Map（操作卡）

> 每单的修复入口假设与验证路径。问题原文/复现/时间线权威在 tickets 工单（EN-xx，brief=false 可取全量），此处只放执行时要随手查的增量信息。

## EN-24 / EN-25 · `_metadataCache.outputShapeKey is not a function`

- **已知事实**（工单 body 实录，不重复展开）：限定 `engram_projects doc_search` + `engram_todos list/add` 路径；memory/assets/wiki 同期正常；渐进扩散且间歇（今晨本会话两域全程正常）
- **管辖边界**（拍板 2026-10-01）：只查 engram 仓库内——错误 JS 形态仅作线索保留（（推断）疑似经外部网关层，但外部不归本线修）；engram 侧排查无果时出具证据、工单转出/关闭
- **验证路径**：定位修复后，连续跑 ① `projects doc_search(project_name=pi-config, query=技能体系)` ② `todos list`（无参）③ `todos add`（打草稿号即删）各 ≥3 次，无异常即过
- **注意**：EN-25 为重复单（同时刻提重），修复合并到 EN-24 追踪
- **结论（2026-10-01）**：外部归因关单——三点证据见 EN-24 resolution（JS 错误串 engram 全库零命中 / 服务端路径零提交 / 9 连调用全绿自愈）；EN-24 resolved，复开条件已留痕（再复现且服务端日志有对应 5xx/panic 才重开）

## EN-10 · doc_patch replace_text 静默失败

- **风险等级**：数据丢失级——修复前纪律：档案/文档维护禁用 replace_text（本树 README 已写）
- **修复入口假设**：doc_patch 的 anchor 匹配 / 写入回执链路（归属 engram 则在 server/mcp projects 域路径）
- **结论（2026-10-01，`576e57c`）**：病根=mcp 层 `dp.content.unwrap_or("")`——content 缺失/空时静默把原文替换为空串且回执照报成功（工单 10 次递减完全吻合）。修复：content 缺失/空显式拒绝 + 回执补 old_chars/new_chars/doc_chars + hint 明示回读验证。回归 5 用例全绿；**replace_text 禁用纪律解除**（见本线 README）。EN-10 resolved。
- **验证路径**：构造 patch → 执行 → **doc_get 回读逐字比对**（不能只看回执）；构造必然失配的 anchor → 必须显式报错

## EN-12 · doc_get 长文档零产出

- **结论（2026-10-01，`cac3111`）**：无法复现——按工单规格造 7280 字符文档实测，MCP doc_get 与 HTTP GET 双通道全文无损、无 content_omitted（projects 域 grep content_omitted 零命中，疑似与 project_get 索引模式混淆）。回归用例 `doc_get_long_content_untruncated_both_channels` 固化。EN-12 resolved；max_chars/meta_only 参数化如需另立（涉 schema+golden 快照）。

## EN-11 · 十域主键参数名不统一

- **拍板（2026-10-01）**：方案 B 报错指路增强（弃 A 别名兼容/C 全量统一）——与 D7 受控宽容哲学一致，宽容不掩盖
- **结论（2026-10-01，`95a115d`）**：dispatch::from_args 严格解析失败时比对原始键集与 schema properties，missing field 场景报错点名「你传入的这些参数本操作不认识：[job_id]」——打回即学会。回归 2 用例（错名点名 + 合法无提示且 D7 宽容解析不受影响）+ golden mcp_surface 字节级稳定（零 schema 变更）。EN-11 resolved。**注意：生产实例需重新部署后生效。**
