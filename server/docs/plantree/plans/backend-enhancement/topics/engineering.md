# Topic: 工程化健壮性

覆盖 R4/R5/R10/R11/R12/R15/R16/R18。核心问题：功能已经很全，但可观测性、容错、运维面还偏薄。

## 现状盘点

- 可观测：`tracing` 结构化 JSON 日志 + `job_events`（LLM 调用/进度），**无 metrics、无追踪导出、无告警**。
- 容错：jobs 有指数退避+抖动+僵尸回收+dead 复活；LLM 调用**无熔断、无 Retry-After**。
- 一致性：多表写（cascade_delete、ingest 的页面+链接+索引）**无事务包裹**。
- 配置：embedding 维度硬编码 1024；Runner 并发全局 4 无 per-kind 限流。
- 运维：无定时维护、无备份、无死信告警。

## R4 多表操作加事务

- 现状：`cascade_delete_source` 依次「删摘要页 → 摘共享页源 → 清 dead link → 删 source 行」，任一步失败留下半删状态。
- 目标：包进 `sqlx` 事务（或至少可回滚的步骤化 + 失败补偿）。
- 依据：error-handling 标准「单点边界 + 不吞错 + 一致性」。

## R5 LLM 熔断 + 429 退避

- 现状：`OpenAiCompatProvider` 直调，429/5xx 归 `Transient`，由 jobs 通用退避重试；**不读 Retry-After，无熔断**。
- 目标：
  - 429 读 `Retry-After` 头，退避对齐服务端节奏。
  - provider 级熔断器（连续失败 N 次 → 打开 → 半开试探），防重试风暴打爆上游。
- 依据：error-handling 标准「重试策略 + 熔断器」。

## R10 可观测性

- 现状：只有日志，出问题靠翻 job_events。
- 目标：
  - `/metrics` 端点（Prometheus）：job 队列深度、LLM 调用数/延迟/用量、检索延迟、错误率。
  - 关键 LLM 调用加 span（tracing 导出），可追踪一次蒸馏/ingest 的完整耗时。
- 依据：observability 标准「线上问题能查得出来」。

## R11 embedding 维度配置化

- 现状：`dimensions: Some(1024)` 遍布 provider/memory/pipeline/wiki，换 embedding 模型要改代码 + 迁移 vector 列。
- 目标：维度进配置（settings 或 env），vector 列宽对齐配置。
- 依据：source-driven 实践「配置与代码分离」。

## R12 per-kind 并发控制

- 现状：`RunnerConfig.concurrency=4` 全局，pipeline.rs 注释自认「单用户规模下解析快，避免互相挤死」。
- 目标：按 job kind 分池（如 LLM 密集的 distill 低并发、CPU 密集的 parse 高并发），避免一个慢任务占满全局。
- 依据：performance 标准「按需调用、限流」。

## R15 定时维护

- 现状：consolidate 手动/防抖，lint 手动，无定时 health。
- 目标：内置定时器（或依赖 PG 定时）跑 consolidate + lint + 健康检查，结果进 job_events。
- 依据：Karpathy 三操作里 lint 是常态操作，不该纯手动。

## R16 数据导出/快照

- 现状：无备份机制，数据全在 PG。
- 目标：API 触发 PG dump 或结构化导出（memory/wiki/knowledge 全量），可恢复到新实例。

## R18 死信告警

- 现状：dead job 靠 UI `revive`，无人盯。
- 目标：dead/failed 任务计数上 `/metrics`，超阈值告警（webhook/log）。
