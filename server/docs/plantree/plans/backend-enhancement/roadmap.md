# Roadmap：后端优化 + 功能清单

优先级依据：单用户定位、价值/成本比、评论区实战验证。P0 = 高价值低成本，立即受益；P1 = 中价值中成本；P2 = 增强项。

## 落地记录

- **2026-08-26：P0 五项（R1~R5）全部落地**，独立审计通过（goal `mt9ydklt-ze8023`）。
  R1 `core/unified.rs` + `POST /search`；R2 `tsv_query_smart` 覆盖全部 4 个 FTS 调用点；R3 context_pack L1 走 `search_atoms`；R4 cascade 全写操作包事务；R5 `CircuitBreaker` + `Retry-After`（含 HTTP-date、HalfOpen 单试探）。`cargo test --workspace` 66 passed。

## 新增（2026-08-28 llm-audit 发现，未排期）

来源：[llm-audit.md](../../../llm-audit.md) L1~L15。**P0×2**：L1 provider 创建零输入校验（空 name/非法 base_url/能力枚举不查；重复 name→503 retryable 误分类；embedding-only 默认 provider 的 chat 模型回退陷阱——or_else(first) 会选嵌入模型当 chat 用）、L2 provider 无更新/删除端点+密钥无版本化（配错 key 只能改库；换 master key 全部密文变砖无重加密路径）。**P1×4**：L3 多默认 provider 无约束（resolve LIMIT 1 无 ORDER 任意取）、L4 routing PUT 零校验+幽灵 provider 静默跳过（typo purpose/错名路由静默落默认，无 warn）、L6 用量记账覆盖不全（grep 实证：knowledge 嵌入与三域检索查询嵌入全部绕过 record_usage——面板系统性低估）、L10 master key 占位符陷阱（"00"*32 缺省下建的密文与后续真实密钥不兼容且无预警）。**P2×5**：L5 test 探针幽灵代码+记账不对称、L7 usage 固定 30 天窗口、L8 usage 端点占位 cipher、L9 熔断器吞永久错误信号（401×5 后熔断打开报 Transient，根因被掩盖）、L15 max_tokens=1 对 reasoning 模型误报。关联：L1←K 校验族（K5/K10 模式）、L4←K1/W1 静默回退族、L6←R10 前置、L9←R5 副作用需一并回归。建议起点：L1+L3（创建路径，半天级）→ L4 → L2（U/D+re-encrypt）→ L6（记账门面）→ L10。

## 新增（2026-08-27 wiki-audit 发现；W1~W8 已于同日修复 ✅）

来源：[wiki-audit.md](../../../wiki-audit.md) W1~W14。**P0×3**：W1 失败重试三重死锁（幂等键墙+冲突路径自删原料文件[with_extension 同路径语义 rustc 实测]+generate 无独立重试；E2E 实测 JSON 抖动脆点的根因确认；=K1 同构且更糟）、W2 tsv 三写三标+检索无向量通道（LLM 页只嵌 slug→内容词搜不到；embedding/HNSW 建而不用）、W3 嵌入失败静默跳过 tsv（页面从检索彻底消失，比 K4 更彻底）。**P1×5**：W4 'failed' 态是幽灵（全代码无人写，卡 processing 误导）、W5 级联删除幽灵边（wiki_links 残边+源重叠权重不重算）、W6 并发 generate 竞态（=G1 具体化：slug UNIQUE 撞车/版本互相覆盖）、W7 log 页无界增长、W8 dead link 清理不吃 [[slug|alias]] 形式。**P2×6**：W9 reingest 无幂等键+review 重复、W10 archive_query slug 不校验、W11 摘源不 bump version、W12 human 提案只活事件流（应进 review_items）、W13 frontmatter 垃圾字段+系统页 title 不可更、W14 全提案轮不重算+lint N+1。关联：W1←K1 修法平移+B5 实录场景（wiki_generate JSON 抖动）、W2←G4 查询侧缺口、W3←B2/K4/K8 嵌入静默失败家族、W6←G1 并发去重、W7←G5 log 设计缺口。建议起点：~~W1→W2→W3→W4/W5~~（已全部落地，2026-08-27 goal mtb8xsy2：workspace 88 tests + E2E 13/13 全绿，含 0014 迁移与 api 启动 tsv 补数；剩 W9~W14 P2 按需）。

## 新增（2026-08-27 knowledge-audit 发现；K1~K9 已于同日修复 ✅）

来源：[knowledge-audit.md](../../../knowledge-audit.md) K1~K16。**P0×5**：K1 sha 幂等墙无状态过滤+失败全 Permanent（一次网络抖动永久卡死，与 wiki sha 墙同构）、K2 enqueue 结果被 .ok() 吞（文档永卡 pending）、K3 SSRF 代理旁路（HTTPS_PROXY 下私网校验全跳）、K4 embed 短响应 NULL 向量+embed_failed=false 双重静默（=B2 同类）、K5 未知二进制默认按文本（mojibake 入库）。**P1×3**：K6 并发 sha 竞态 503、K8 embed 失败永久降级无恢复入口、K9 URL 瞬态错误归 Permanent。**P2×8**：K7 空 token 查询静默零召回（单字/纯标点无声返回空，三域共用，tokenize 一处修；勿用哨兵——'!' 实测报 no operand 错）、K10 游标非唯一、K11 chunk 重跑残留、K12 extracted.txt 泄漏、K13 检索嵌入降级无日志、K14 chunks 500 截断、K15 URL 内容级去重、K16 大文件成本护栏（可并入 R15）。关联：K7←R2 延续、K16←R15 合并、K13←R10 挂载、K4 参照 B2 修法。建议起点：~~K2+K6 → K7 → K1/K9~~（已全部落地，2026-08-27 goal mtb43ztv：workspace 81 tests + E2E 13/13 全绿；剩 K10~K16 P2 按需）。

## 新增（2026-08-27 memory-audit 发现；B1/B2/B3/B5/B6/B9/B10 已于同日修复 ✅）

来源：[memory-audit.md](../../../memory-audit.md) B1~B12。
**已修（2026-08-27，goal mtazbygx）**：B1 extract 分段覆盖率（6000 字符贪心分段+段级事件+prompt v2）、B2 零向量置 NULL+FTS 兜底（含 MockLlm 全零碰撞修复）、B3 persona 证据链分面化（S 编号标注+L0 会话链+prompt v2）、B5 chat_json_retrying 统一、B6 duplicate 改 archived+superseded_by、B9 检索命中回写 hit_count（0013 迁移+search/context_pack 双路径）、B10 stale 降权不自锁（created_at 判龄）。workspace 69 tests + E2E 13/13 全绿。
**未修**：B4（聚类先验排除交叉组）、B7（persona 幻觉校验）、B8（防抖窗口配置化）、B11（organize 孤儿重进 prompt）、B12（persona 版本竞态）——均为 P2 级，按需排期。

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
