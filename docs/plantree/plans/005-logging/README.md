# P005 · 全面日志系统（存储落地）

> 2026-10-01 用户定调：「针对系统的所有功能，进行一整套全面、详细的日志存储落地」——完整、详尽、支持调试和验证。

## 现状盘点（2026-10-01 代码实证）

1. **stdout JSON 日志**：`tracing_subscriber::fmt().json()` + EnvFilter（RUST_LOG，默认 info）——**无文件/无落库**（tracing-appender 零命中）；生产 Docker 下= docker logs，容器重建即失。EN-32 排查时「三篇文档卡 embedding 但查不到当时的 LLM 失败记录」即此缺口实证。
2. **HTTP 层**：tower_http `TraceLayer`（请求日志默认 DEBUG 级——被 info 过滤，**生产实际不可见**）+ R10 metrics（计数/直方图，指标非日志）。
3. **LLM 调用**：仅失败 warn（status + body 前 500 字）；**成功调用零记录**——无 purpose/模型/耗时/token/请求响应轨迹。llm_usage 表只有 token 账没有调用日志。
4. **job 体系**：`job_events` 表是**唯一落库的日志面**（ctx.emit 人工粗粒度事件）。
5. **业务域**（memory/wiki/projects/todos/assets/credentials）：服务层基本无日志，只有零散 warn。

## Scope

全部后端功能的日志存储落地：基础设施（结构化日志落库 + 保留期）+ 请求追踪（request-id 贯穿）+ LLM 全量调用日志 + 业务审计动作 + 查询面（API + 前端页）。
Affected Modules: server/api, server/core, server/llm, server/storage（新迁移）, server/jobs, web（日志页）。

## File Map

- `roadmap.md` — 任务状态唯一权威
- `open-questions.md` — 待拍板细节
- `evidence/` — （待补）

## 关联

- P004（T010 harness）：工具调用审计与本计划审计日志共用基础设施；P006（错误处理）：错误→日志统一反馈层建在本计划 logs 基础设施上
- 红线：credentials 值/token/密码永不落日志（AGENTS.md 凭据纪律）；日志表属新迁移（63→64+，同步 migrations_test 断言）
