# Wiki 域实现审查

> 2026-08-27 · 与 [memory-audit.md](memory-audit.md) / [knowledge-audit.md](knowledge-audit.md) 同规格：Part A 机制全解，Part B 问题清单（每条含代码位置 / 影响 / 修法建议）。
> 审查范围：`crates/wiki-engine/src/` 全部 12 文件 + `crates/api/src/routes/wiki_api.rs` + `migrations/0007_wiki.sql` + 跨 crate（llm_port 错误映射、search tokenize）。
> 行为断言均经源码逐行核对；不确定语义实测验证（`Path::with_extension` 同名替换 = 同路径，rustc 实测）。机制详解另见 [wiki/](wiki/README.md) 六篇模块文档，本文不重复，只写审查视角的链路与缺陷。

## Part A 机制全解（审查视角）

### A1 摄取链与状态机

`wiki_sources` 四态机 `pending → processing → ready | failed`（0007 CHECK）。**全代码无一处写 `'failed'`**——该状态仅存在于约束定义中。两步 job 链：

```
ingest() → enqueue_ingest（sha 幂等：ready 秒跳过；非 ready 走 ON CONFLICT 重置 pending）
  wiki_analyze   → LLM 结构化分析（purpose + index 注入）→ reviews/purpose 建议落库 → 链式入队 generate
  wiki_generate  → LLM 生成 pages[] → 逐页三分支（新建 llm / human 页提案 / LLM 页合并 version+1）
                 → rebuild_links → index/log/overview 系统页 → 嵌入 → relevance 全量重算
```

幂等键：`wiki-analyze-{source_id}` / `wiki-generate-{source_id}`（jobs 表全局唯一，任何状态复用即返回既有）。人工编辑入口 `put_page`（ON CONFLICT origin→human + version+1 + tsv 重嵌）。`reingest()` 死任务重跑入口**无幂等键**。

### A2 页面写入三分支（generate_job）

新建 → `origin='llm'` version=1；`origin='human'` 既有页 → **不覆盖**，提案内容只 emit 进 job_events（`proposal_content`）；LLM 既有页 → `UPDATE content + sources 并集(jsonb_agg UNION) + version+1`。系统页（index/log/overview）由 `upsert_system_page` 幂等重建（ON CONFLICT 只更新 content）。

### A3 图与相关性

`wiki_links`（from/to/weight，PK 双 slug）。`rebuild_links`（ingest 时）重建本批页面出边（weight=3.0 直链）；`rebuild_weights`（ingest 尾声，仅 created+updated>0 时）对**全部**边按 4 信号重算（直链 3.0 + 源重叠 4.0 + Adamic-Adar 1.5 封顶 1.0 + 类型亲和 1.0，下限 0.1），并对「源重叠但无直链」的页面对**无向补双条有向边**。社区：简化单层 Louvain（贪心迁移，O(n) 轮上限）；凝聚度 = 社区内边权和 / 可能对数。洞察 4 类（isolated/sparse/bridge/surprising）现算不落库，dismiss 持久化于 `wiki_insight_dismissals`（稳定键 `kind:slug`）。

### A4 检索

`WikiService::search`：**纯 FTS 单通道**——`wiki_pages.tsv @@ to_tsquery(smart)` + ts_rank 排序（注释声称「FTS + 向量 RRF」但无向量通道；`embedding` 列 + HNSW 索引存在但从不参与查询）。tsv 的写入者有三处、口径不一（见 W2）。`search_with_purpose` 包装 purpose 注入。

### A5 治理

级联删除（R4 已包事务）：sources[] 含目标 → source 型独源页整删 / 共享页摘源；content 中 dead `[[link]]` 文本清理；删 wiki_sources 行；index 重建在事务外。lint 5 规则只报告。review：analyze 期 LLM flag（4 预定义 kind 防幻觉）落 `wiki_review_items`，resolve/dismiss 带 action 标签。

---

## Part B 问题清单

> 按严重度排序。**P0=正确性/数据一致性，P1=质量/健壮性，P2=增强**。
> 与 gap-analysis G1~G6 及 B/K 系列的关联逐条标注。

> **修复状态（2026-08-27）**：W1~W8 已修复（goal mtb8xsy2；workspace 88 tests + E2E 13/13 全绿）——
> W1 死锁三锁全解（删自删行+状态感知重入队[analysis 从失败 job payload 直发 generate]）、W2 tsv 改嵌 title+content+检索向量 RRF 通道+启动存量补数、
> W3 tsv 与嵌入解耦+K4 守卫移植+失败事件、W4 0014 error 列+Permanent 统一 mark_failed+自愈、W5 级联删边+无据边回收+权重重算、
> W6 页写入单语句 UPSERT（human 保护进 WHERE）+rebuild_links 事务化、W7 log 截 500 行、W8 alias 结构化移除。
> 未修：W9~W14（P2 级，按需排期）。

### P0

**W1 · 失败重试三重死锁——幂等键墙 + 冲突路径自删原料文件 + generate 无独立重试（E2E 实测根因确认）**
- 位置：ingest.rs:90-96（幂等键 `wiki-analyze-{id}`，jobs 幂等查询无状态过滤——任何终态 job 复用即返回既有，不创建新 job）；ingest.rs:75-88（sha 冲突路径：写 `{real_id}.md` 后第 81 行 `remove_file(path.with_extension("md"))` ——**rustc 实测 `with_extension("md")` 于 `.md` 路径返回原路径，此行删掉的是刚写的原料文件**，`path` 在 77 行被 shadow，注释「no-op clarity」与实际行为相反）；ingest.rs:172-178（generate 仅由 analyze 成功路径链式入队，analyze 曾成功而 generate 失败时无任何再触发路径）。
- 影响：一次 wiki_generate JSON 抖动（E2E 已实测 MiniMax-M3 长页输出在 2172 列断裂）后，重提交同 sha ① 幂等墙返回旧 analyze job 不再链 generate；② 即使绕过，原料文件已被第 81 行删除，revive 重跑 analyze 直接 Permanent「读原料失败」；③ reingest 端点可入新 job 但读不到文件。三锁叠加 = 该 sha 永久死锁，只能变文本换 sha（用户不可发现）。= **K1 同构 + 更糟**（K1 至少没删文件）。
- 修法：① 删除 81 行（或改为 `remove_file` 第一写的 `{id}.md`——但它 76 行已删，直接删行）；② analyze 幂等键带内容版本（`wiki-analyze-{id}-{sha 前缀}`）或复用 K1 的状态感知自愈：enqueue 前查 analyze/generate job 状态，仅当全终态且 source 非 ready 才重入队 generate；③ K1 式 `enqueue` 失败分类（Transient 网络类走队列退避）。

**W2 · tsv 三写三标 + 检索无向量通道——LLM 页内容词根本搜不到**
- 位置：ingest.rs:574-578（`write_embeddings`：`tsv = to_tsvector('simple', tsv_text(slug))` ——**只嵌 slug**）；service.rs:156-170（`put_page`：嵌 content）；service.rs:333-343（`archive_query`：嵌 content）；service.rs:239-255（`search`：纯 FTS，注释「FTS + 向量 RRF」名不副实，embedding/HNSW 建了索引从不查询）。
- 影响：wiki 页面的绝对多数（LLM 生成）FTS 只能按 slug 命中——正文词、概念词、对比结论全部搜不到（例：`pgvector-vs-milvus` 页搜「成本」「延迟」零命中）。与「按内容检索知识」的域定位直接冲突。统一检索 `/search`（R1）的 wiki 通道同受此限。这是三域审查中最大的单点召回缺陷。
- 修法：① `write_embeddings` 改嵌 `tsv_text(&format!("{title}\n{content}"))`（与 collect_page_texts 同口径）；② 补数：一次性 SQL `UPDATE wiki_pages SET tsv = to_tsvector('simple', ...)` 重建存量；③ search 加向量通道（RRF 融合，参照 knowledge mod.rs 现成模式）或至少修正注释。

**W3 · 嵌入失败静默跳过 tsv——页面完全从检索中消失（比 K4 更彻底）**
- 位置：ingest.rs:345-351（`if !texts.is_empty() && let Ok(emb) = llm.embed(...)` ——embed Err 时 `write_embeddings` 整个不执行，tsv 写入捆绑在嵌入成功路径里）；ingest.rs:571-572（`embeddings.get(i)` None 时静默跳过该页——短响应旁路，K4 同款）。
- 影响：嵌入 provider 故障期间的 ingest：页面正常创建/更新（用户看到页面在），但 tsv 与 embedding 双 NULL → FTS 搜不到（W2 之下 tsv 是唯一通道）→ **页面从检索中彻底消失**，无标志无事件无恢复入口（对比 knowledge 的 embed_failed 标志位）。重启 provider 后也无补嵌路径（页级 re-embed 不存在）。
- 修法：① tsv 写入与嵌入解耦——generate 阶段无条件写 tsv（嵌 content 口径，随 W2 一起修）；② embed 失败 emit 事件 + 页面级补嵌入口（参照 K8 的 re-embed 模式）；③ 短响应/维度不符整批拒绝（K4 守卫移植）。

### P1

**W4 · analyze/generate 失败后 wiki_sources 永卡 processing——`'failed'` 状态是幽灵**
- 位置：ingest.rs:115（analyze 置 processing，此后全代码无 UPDATE 写 'failed'；0007 迁移定义了该状态但零使用）。generate 失败同样不回写。
- 影响：失败源在 `/wiki/sources` 列表里永远显示 processing（误导「还在跑」）；与 W1 死锁叠加后，用户唯一的 UI 线索也是错的。= K1 的失败可见性半部（K 系列修法可平移）。
- 修法：analyze/generate 的 Permanent 失败路径统一 `mark_failed`（0007 迁移的 wiki_sources **无 error 列**，需 0014 补列或复用 job error——建议补列，列表页可直接展示失败原因）+ K1 式重提交自愈。

**W5 · 级联删除留幽灵边——wiki_links 残边 + 源重叠权重不重算**
- 位置：cascade.rs（全文无 `DELETE FROM wiki_links WHERE to_slug IN (...)` 或 `from_slug IN`——删页只清 content 里的 `[[link]]` 文本，**边表里的边原样残留**）；cascade.rs 尾部（摘源后不调 `rebuild_weights`——被摘源页对的「源重叠补边」（weight 4+，relevance.rs:158-169 无向双插）继续以旧关联存在）。
- 影响：删源后图视图（graph 端点）出现指向不存在页面的边（前端 sigma 渲染悬空节点/边）；洞察（surprising_connection 用 wiki_links 权重）基于幽灵关联误报；共享页被摘源后与同源页的 4.0 边残留——「删除」语义不完整。E2E 只断言了 content 清理数（cleaned_links），未覆盖边表。
- 修法：cascade 事务内补 `DELETE FROM wiki_links WHERE from_slug = ANY($deleted) OR to_slug = ANY($deleted)`；commit 后对 updated_shared 页调 rebuild_weights（或直接删除摘源后不再成立的源重叠边）。

**W6 · 并发 ingest 竞态——slug UNIQUE 撞车与版本互相覆盖（= G1）**
- 位置：ingest.rs:270-336（generate 页写入：check-then-act——两个并行 generate 各自 `SELECT id, origin WHERE slug=$1` 都得 None → 双 INSERT 撞 `slug UNIQUE` → 一方 Retryable 失败整体重跑；都得 Some → 双 UPDATE version 各 +1，**后写覆盖先写内容，前一批的 content 更新丢失**）；ingest.rs:416-444（rebuild_links 的 DELETE+INSERT 无事务包裹，与另一 generate 的链接重建交错可留半态）。
- 影响：单用户手动操作概率低，但「重试中的旧 job 与新提交的 job 并行」（visibility_timeout 后 reap 重排）是真实场景。= gap-analysis G1 的具体化。
- 修法：页 UPSERT 单语句化（`INSERT ... ON CONFLICT (slug) DO UPDATE ... WHERE wiki_pages.origin='llm'`——human 页保护语义保留在 DO UPDATE 的 WHERE 里）；rebuild_links 包事务。

**W7 · log 系统页无界增长——每次 ingest 全量重写**
- 位置：ingest.rs:457-476（`prev_log + 新行` 整页重写，无行数上限；wiki_pages.content text 无限长）。
- 影响：千次 ingest 后 log 页数百 KB，每次 ingest 全量读+全量写；list_pages 虽排除 log，但 get_page('log') 与 rebuild_index_page 的 `NOT IN ('index','log')` 查询都要扫它。慢性膨胀。
- 修法：截最近 N 行（如 500）+ 全量历史已由 job_events 承担（注释自认）；或 log 改为视图/独立表。

**W8 · dead link 清理不吃 alias 形式——`[[slug|别名]]` 残留碎片**
- 位置：cascade.rs:105-108（`cleaned.replace("[[{d}]]", "")` 精确串匹配——markup.rs:15 的 alias 语法 `[[slug|display]]` 不匹配，清完留下 `|display]]` 裸碎片在正文里）。
- 影响：删源后共享页正文出现 markdown 碎片（渲染成奇怪文本）。E2E 用例只造了无 alias 的链。
- 修法：按 `extract_wikilinks` 的解析规则做正则替换（`\[\[{slug}(\|[^]]*)?\]\]`），或复用 markup 抽出的结构化位置重写。

### P2

**W9 · reingest 无幂等键 + analyze 重跑重复造 review items**
- 位置：service.rs:272-281（enqueue 无 with_idempotency_key，连点 N 次排 N 个 analyze）；review.rs:45-76（create_items 无去重，重复 analyze 对同一 flag 再插一行）。
- 修法：键 `wiki-reingest-{id}-{uuid}`；create_items 按 (source_id, kind, title) 幂等。

**W10 · archive_query 不校验 slug——`query-{title}` 可含空格/斜杠入库**
- 位置：service.rs:331（`format!("query-{title}")` 直接 INSERT，markup::is_valid_slug 未调用；wiki_pages.slug 无格式 CHECK）。title 含空格 → slug 带空格 → extract_wikilinks 永不承认它（is_valid_slug 过滤）→ 天然死孤儿 + lint 报 orphan。
- 修法：slug 归一化（空白→`-`）+ 校验，非法回 BadRequest。

**W11 · 摘源不 bump version——共享页变更无版本痕迹**
- 位置：cascade.rs:72-85（UPDATE 刷 updated_at，version 不动）。摘源是内容语义变更（sources 数组变了），版本历史却无记录。
- 修法：`version = version + 1` 一并写。

**W12 · human 页提案只活事件流——与 review 体系脱节**
- 位置：ingest.rs:302-315（提案内容 emit 进 job_events 的 payload，**不落 wiki_review_items**）；service.rs:229-237（apply_proposal 是通用 put_page 包装，内容靠 UI 从事件流手工捞回传）。
- 影响：事件流当数据库用——job_events 被清理/滚动后提案内容丢失；ReviewQueue UI 里看不到这批提案（它只读 review_items）；「提案→审核→合入」闭环断在中间。G 系列异步人审的具体缺口。
- 修法：提案落 review_items（kind 复用 `create_page` 或新增 `update_page`，payload 带 slug+content），resolve 时带出内容供 apply。

**W13 · frontmatter 垃圾字段 + 系统页 title 不可更新**
- 位置：ingest.rs:281（`origin_if_new` 进 frontmatter——无任何消费者，origin 是独立列）；ingest.rs:523-544（upsert_system_page ON CONFLICT 只 SET content，title/page_type 永远是首次值）。
- 修法：删字段；系统页 ON CONFLICT 补 `title = $3`。

**W14 · rebuild_weights 全提案轮不重算 + lint stale_source N+1**
- 位置：ingest.rs:360-366（`created + updated > 0` 才重算——本轮全被 human 保护拦成提案时，链接权重与 overview 停留旧值）；lint.rs:102-118（循环内逐 source count 查询）。
- 修法：proposals>0 也重算；lint 用一次 JOIN 聚合。

---

## 与既有系列的关联矩阵

| 本清单 | 对应 | 关系 |
|---|---|---|
| W1 | K1（已修）+ **B5**（已修） | K1 同构三连（幂等墙+错误分类+自愈）但多一层文件自删；B5 实录「E2E 实测 wiki_generate 就撞过 JSON 抖动」正是 W1 的触发场景——B5 修的是 memory 侧统一走 retrying，wiki 侧的链式死锁由 W1 补全收口 |
| W2 | **G4** | 同属查询侧能力缺口：G4=按 page_type 路由查询缺失，W2=向量通道缺失 + tsv 口径断裂（二者叠加使检索只余 slug 精确匹配） |
| W3 | **B2**（已修）+ K4/K8（已修） | 嵌入静默失败家族三部曲：B2 零向量/NULL 仲裁旁路 → K4 短响应 NULL+false 双静默 → W3 连 tsv 都不写（最彻底：页面从检索整体消失） |
| W4 | K1 可见性半部 | mark_failed + 自愈平移 |
| W6 | **G1** | gap-analysis 并发去重缺口的具体化 |
| W7 | **G5** | 同属 log 系统页设计缺口：G5=只记 ingest（内容不完整），W7=全量重写无界增长 |
| W12 | 无直接 G 对应 | 异步人审机制本身已实现（4 预定义 kind 防幻觉），W12 是 human 页提案未进 review_items 的闭环缺口 |
| W5/W8/W9/W10/W11/W13/W14 | 新发现 | wiki 特有 |

修复优先级建议：**W1（三重死锁，用户可感知的「卡死」）→ W2（召回根基，一次 SQL 补数+一行改嵌）→ W3（tsv 解耦，随 W2 顺带）→ W4/W5（失败可见性+删除完整性）→ W6~W8 → P2 按需**。W1/W2/W3/W4 高度相关，适合一个 goal 内连续修。
