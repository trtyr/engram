# P006 · 全面错误处理机制

> 2026-10-01 用户定调：与 P005 日志系统配对的地基工程——「错误处理跟日志，唯一的目的就是为了让我们能够更好地调试」。

## 用户痛点 → 现状缺口（2026-10-01 实证）

| 痛点 | 现状缺口 |
| --- | --- |
| ①不知道哪里出了问题 | 无 request-id 贯穿（P005 T002 补）；错误发生点无位置/操作上下文字段 |
| ②不知道是什么样的错误 | 错误信息是 format! 自由字符串（如「文档 {doc_id} 不存在」）；无错误码体系、无结构化上下文（资源/ID/约束）；跨层 wrap 时上下文常被 map_err 丢掉 |
| ③外部错还是内部 bug | 无归因分类学——现有 JobError::Permanent/Retryable 是 job 重试视角，LlmError::Transient 是熔断视角，都不是「外部输入/上游服务/网络/认证/内部 bug」的归因视角；EN-24 排查时归因外部花了一整天，就是因为没有分类字段 |

## 现状快照

- 各 crate 各自的 thiserror enum（WikiDocumentError / ProjectError / JobError / LlmError / mcp_err ErrorCode...），互不相通
- 错误→日志无统一反馈层：有的 warn 有的静默有的 map_err 丢弃（EN-24 排查时的痛点重演）
- MCP/HTTP 边界的错误映射各自为政（mcp_err / ke() / map_err(|e| ...) 散落）

## Scope

统一错误模型 + 错误码体系 + 传播链纪律 + 错误→日志统一反馈 + 边界映射规范化 + 错误码文档。
Affected Modules: server/core（模型）, server/storage, server/llm, server/jobs, server/mcp, server/api, server/distill, web（错误展示）。

## File Map

- `roadmap.md` — 任务状态唯一权威
- `open-questions.md` — 待拍板细节

## 关联

- P005（日志）：错误→日志统一反馈层建在 P005 T001 的 logs 基础设施上；request-id 贯穿共用
- P004（harness）：harness 的工具失败归因（外部/内部）直接消费本计划的分类学
- 纪律：对外错误脱敏内部细节（路径/SQL/密钥痕迹）；错误码全表文档化进 engram projects 接口面
