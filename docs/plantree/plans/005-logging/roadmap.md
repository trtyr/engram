# P005 Roadmap

> 任务状态唯一权威。现状盘点见 README。

## In Progress

（空——2026-10-02 六任务全落地）

## Next

（空——全部任务已结）

## Deferred

（空）

## Done

- [x] 现状盘点 + 差距定性（2026-10-01，见 README）。
- [x] **T001 · 日志基础设施**（9b99c55）：0064 logs 表（3 索引）+ PgLogLayer（mpsc 8192 try_send 不阻塞）+ 批量 writer（64 条/500ms）+ 保留期清理（info 30 天/debug 7 天，env 可调）。
- [x] **T002 · request-id 贯穿 + HTTP 请求日志**（cd51c9f）：tokio task_local REQUEST_ID + request_id_mw（生成/回显 x-request-id）+ writer 提升到列（idx_logs_request_id 可查）。
- [x] **T003 · LLM 全量调用日志**（a6f1719）：record_usage 单点打点（provider/model/purpose/tokens/latency/job_id）覆盖全部 4 调用点 + chat/embed 失败 warn。
- [x] **T004 · 业务审计动作**（2755a0d）：6 审计点（login 成败/credentials put+delete/apikey create+revoke+batch）全带 audit=true。
- [x] **T005 · 查询面**（e1c5da4）：GET /logs（admin 门，level/q/request_id/since/until/audit 过滤+分页）+ 前端 Logs 页（过滤器+表格+10s 自刷+分页）。
- [x] **T006 · 覆盖面补齐**（0c64198）：wiki_docs 管线 4 个 debug 轨迹点（parse/chunk/embed 起止+计数）。
