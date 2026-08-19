# Topic — 任务系统

一切长操作（蒸馏、摄取、ingest、同步、consolidation）都是 job。PG-backed，无外部队列。

## 状态机

```text
pending → running → succeeded
              ↘ failed（retryable 且 attempts<max → pending 重排）
              ↘ dead（重试耗尽；人工可复活重跑）
```

- 抢任务：`SELECT ... FOR UPDATE SKIP LOCKED` 批量领取；`locked_by/locked_at` 心跳
- 崩溃恢复：running 超 `visibility_timeout`（默认 5min，按 kind 可配）→ 回收重排，attempts+1
- 幂等键：`idempotency_key` 唯一约束，API 层 Idempotency-Key 透传，重复提交返回既有 job

## 重试策略

| 错误类 | retryable | 退避 |
|---|---|---|
| LLM 429 / 5xx / 超时 | ✅ | 指数+抖动 200ms→30s，≤3 次 |
| 网络抖动（DB 短暂失联） | ✅ | 同上 |
| 输出 JSON 解析失败 | ✅（1 次，附修复指令） | 固定 1s |
| 参数/校验/逻辑错误 | ❌ 直接 failed | — |

## 并发与调度

- worker 池：tokio 任务，全局并发上限（默认 4）+ 按 kind 上限（embedding 类 8，其他 2）
- 定时任务：`due_at` 列 + 轮询（consolidate 每周）；不引入外部 cron
- 链式：job 完成回调可入队下游 job（extract→arbitrate→organize→persona 用此机制）

## 事件与可观测

- `job_events` 追加：状态转移、进度（n/total）、LLM 调用摘要（model/tokens/耗时）、错误详情
- `GET /jobs/:id/events` 支持 SSE（前端实时进度条）
- 结构化日志：tracing + JSON 输出，correlation：每 job 携带 job_id 贯穿日志链

## 测试约定

- 状态机/重试/恢复全部单元测试（时钟 mock）
- 集成测试用 testcontainers PG 验证抢占、恢复、幂等
