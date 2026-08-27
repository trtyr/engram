# Roadmap：后端优化 + 功能清单

优先级依据：单用户定位、价值/成本比、评论区实战验证。P0 = 高价值低成本，立即受益；P1 = 中价值中成本；P2 = 增强项。

## 落地记录

- **2026-08-26：P0 五项（R1~R5）全部落地**，独立审计通过（goal `mt9ydklt-ze8023`）。
  R1 `core/unified.rs` + `POST /search`；R2 `tsv_query_smart` 覆盖全部 4 个 FTS 调用点；R3 context_pack L1 走 `search_atoms`；R4 cascade 全写操作包事务；R5 `CircuitBreaker` + `Retry-After`（含 HTTP-date、HalfOpen 单试探）。`cargo test --workspace` 66 passed。

## 新增（2026-08-27 memory-audit 发现，未排期）

来源：[memory-audit.md](../../../memory-audit.md) B1~B12。P0 级三条：B1 extract 长会话静默丢段（=R7）、B2 零向量/NULL embedding 仲裁旁路、B3 persona 证据链全量共享。P1 级五条含 B5 arbitrate/consolidate 裸调 chat_json 无重试、B6 duplicate 物理删除丢溯源、B9 hit_count 语义残缺致 stale 误伤（=R8 依赖）。修 P1 批（R7/R8）时应连同 B2/B3/B5 一并处理。

## P0 —— 高价值，低成本

| ID | 项 | 现状 | 目标 | 依据 |
|---|---|---|---|---|
| R1 | 跨域统一检索 | memory/knowledge/wiki 三套检索各自为政（`search_atoms`/`search_scenarios`/wiki FTS/chunk 检索分散） | 一个 `/search` 入口，RRF 融合三域，一次问全 | 评论区「index 有损瓶颈」教训的延伸 |
| R2 | tsquery 召回优化 | `tsv_query` 用 `&` 连接全部 token（AND 全命中，过严） | OR 兜底 / phrase 匹配 / 命中数可调 | `tokenize.rs:tsv_query` |
| R3 | context_pack L1 相关性 | L1 补充按 `hit_count DESC, confidence DESC` 排序，非语义相关 | L1 也走 embedding 检索（与 query 相关） | `memory.rs:context_pack` |
| R4 | 多表操作加事务 | cascade_delete 删页/删源/清链接多步无事务，中途失败不一致 | 关键多表写包进事务 | `cascade.rs`、`ingest.rs` |
| R5 | LLM 熔断 + 429 退避 | provider 直调，429 只标 transient 重试，无 Retry-After | 熔断器 + 按 Retry-After 退避 | `provider.rs`、error-handling 标准 |

## P1 —— 中价值，中成本

| ID | 项 | 现状 | 目标 | 依据 |
|---|---|---|---|---|
| R6 | LLM rerank | 混合检索后无重排序 | 可选 LLM 重排（top-k 内精排） | qmd/llm_wiki 重排 |
| R7 | extract 长会话覆盖率 | 一批会话全拼进一个 LLM 调用，长会话超上下文丢中间段 | 有界跨度 + 覆盖率计划（每段显式处理） | 评论区 XBlueSky 教训 |
| R8 | 命中反馈 | `SearchHit.hit_count` 存在但 search 从不更新 | 命中即 `hit_count++`，常用记忆浮上来 | `memory.rs`、`search` |
| R9 | chunk 上下文扩展 | chunk 检索返回无父文档/相邻 chunk | 带 parent title + 相邻 chunk | `chunking.rs`、knowledge 检索 |
| R10 | 可观测性 | 仅 tracing 日志 + job_events，无 metrics | Prometheus metrics + 追踪 span 导出 | observability 标准 |
| R11 | embedding 维度配置化 | 硬编码 `dimensions: Some(1024)` 遍布 | 配置化，换模型不改代码 | `provider.rs`/`memory.rs`/`pipeline.rs` |
| R12 | per-kind 并发控制 | Runner 全局 concurrency=4，无 per-kind 限流 | 按 job kind 分池限流 | `pipeline.rs` 注释自认 |

## P2 —— 增强项

| ID | 项 | 现状 | 目标 | 依据 |
|---|---|---|---|---|
| R13 | Read Sources Only | 无「仅原文回答」开关 | 检索可切「只信原文」模式，对抗幻觉固化 | llm_wiki 安全阀 |
| R14 | pin 存活增强 | `origin=human` 整页保护（简化版） | 记录修正意图 + 小节锚定 + 重编译核对 | 评论区 huachen-wang 教训 4 |
| R15 | 定时维护 | consolidate/lint 仅手动或防抖触发 | 定时 lint + consolidate + health check | Karpathy 三操作 |
| R16 | 数据导出/快照 | 无备份机制 | 数据导出/快照（PG dump 或 API） | 运维 |
| R17 | Wiki↔Memory 互操作 | 两域隔离 | wiki 页面可蒸馏回记忆 / 记忆可喂 wiki | 跨域编排 |
| R18 | 死信告警 | dead job 靠人工 revive，无告警 | dead/failed 任务上报告警 | `queue.rs:revive` |

## 展开

- [retrieval.md](topics/retrieval.md) — R1/R2/R3/R6/R8/R9/R13
- [distillation.md](topics/distillation.md) — R7/R14/R17
- [engineering.md](topics/engineering.md) — R4/R5/R10/R11/R12/R15/R16/R18
