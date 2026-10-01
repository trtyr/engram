# P002 Roadmap

> 任务身份/状态/顺序的唯一权威。问题细节权威在 tickets 工单（EN-xx），此处只追状态。

## In Progress

（空——T001-T005 全部 Done）

## Next

（空）

## Deferred

- **EN-12 参数化诉求**（max_chars/full/meta_only 显式参数）：如需上下文预算控制另立工单（涉 inputSchema + golden 快照同步）；现状全文无损已覆盖 acceptance ①
- **EN-24 复开条件**：_metadataCache 类异常再复现 **且** engram 服务端日志存在对应 5xx/panic 才重开（外部归因已在 EN-24 resolution 留痕）

## Done

- [x] **T001 · 工单 hygiene 三件套**（2026-10-01）：EN-25↔EN-24 + EN-23↔EN-24 relates_to（links 回读确认）· EN-25 archived（comment 留痕）· todos EN-26 done。
- [x] **T002 · `_metadataCache.outputShapeKey` 排查**（2026-10-01）：engram 源码 grep 零命中（JS 形态错误不可能出自 Rust 单二进制）+ 故障时间线内服务端路径零提交 + 本会话 9 次调用全绿（doc_search×3/list×3/add×2/delete×2）→ **外部归因（（推断）pi MCP adapter），EN-24 resolved 关单（resolution 留复开条件），EN-25 随归档**。
- [x] **T003 · EN-10 doc_patch replace_text 静默失败修复**（2026-10-01，`576e57c`）：病根=`dp.content.unwrap_or("")` 把 anchor 原文替换成空串且回执照报成功（工单 10 次递减 5967→5153 完全吻合；「双引号」线索=外部序列化丢 content 的触发器，服务端防御缺失是主病灶）。修复：content 缺失/空显式拒绝 + 回执补 old_chars/new_chars/doc_chars + hint 明示回读验证。回归 5 用例全绿 + clippy -D warnings 绿。EN-10 resolved；replace_text 禁用纪律解除。
- [x] **T004 · EN-12 doc_get 长文档**（2026-10-01，`cac3111`）：按工单规格造 7280 字符文档实测——MCP doc_get 与 HTTP GET 双通道全文无损、无 content_omitted，**无法复现**（projects 域 grep content_omitted 零命中；疑似与 project_get 索引模式混淆）。回归用例固化。EN-12 resolved。
- [x] **T005 · EN-11 十域主键参数名统一**（2026-10-01，`95a115d`，拍板 B）：dispatch::from_args missing field 时点名未识别键（「你传入的这些参数本操作不认识：[job_id]」）——打回即学会，不做自动改名。回归 2 用例 + golden mcp_surface 字节级稳定（零 schema 变更）+ 跨域正确键名实测。EN-11 resolved。**生产实例需重新部署后生效。**
