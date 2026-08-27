# Memory（用户记忆）域实现审查

> 2026-08-27 生成。范围：L0~L3 蒸馏全链 + 检索/上下文包 + 治理面。代码基准：当前 main。
> 结构：Part A 实现机制（怎么做的）→ Part B 问题清单（哪里有问题、建议怎么修）。

---

## Part A：实现机制

### A1. 数据模型（四层）

| 层 | 表 | 关键字段 | 语义 |
|---|---|---|---|
| L0 | `raw_sessions` | `content jsonb`（轮次数组）、`distill_status`（pending/processing/done） | 原始对话，不可变，溯源终点 |
| L1 | `atoms` | `kind`（8 枚举）、`confidence`、`status`（**candidate**/active/superseded/archived）、`source_refs`、`hit_count`、`scenario_id`、`embedding vector(1024)`、`tsv` | 原子事实，一句自包含短句 |
| L2 | `scenarios` | `topic`（≤6字）、`summary`、`body`、`atom_refs`、`version` | 场景知识块（主题归组） |
| L3 | `persona_aspects` | `aspect`（7 枚举）、`content`、`evidence_refs`、`version`、`prompt_version` | 画像分面，**版本化不可变**，每 aspect 取 max(version) 为当前 |

迁移：0005 建表；0011 加 `candidate` 状态与 `scenario_id` 归属列（含两个部分索引：未归组、待仲裁）。

### A2. 蒸馏链（五段 job，条件链式入队）

```
write_session(distill=auto) ──30s 防抖──► extract_atoms ──► arbitrate_atoms ──(有转正)──► organize_scenarios ──(有变动)──► distill_persona
                                              │                                                        │
consolidate（每周自续期，手动 full=true 也触发）┘                                                        └─ evidence: scenarios + atoms refs
```

**extract（`extract.rs`）**：认领全部 pending 会话（`UPDATE ... WHERE distill_status='pending'` 原子认领，失败回滚 pending）→ 全部轮次拼一个编号文本 → LLM 抽取（kind 8 枚举、content ≤120 字硬过滤、confidence clamp、turn_refs 映射回 session_id）→ 落 `candidate` 状态 + 批量 embed → 会话标 done → 链式入队 arbitrate（带 candidate_ids）。

**arbitrate（`arbitrate.rs`）**：每条候选取 top-5 向量近邻（余弦，`ORDER BY embedding <=>`）→ 无近邻直接转正；有近邻交 LLM 三判定：
- `duplicate`：删候选，既有条 `hit_count+1`
- `contradicts`：候选转正 + 旧条 `status='superseded', superseded_by=新id`
- `new`：转正
防幻觉：candidate_id 白名单校验；LLM 漏判的候选**兜底转正**（不让 candidate 滞留）。仅 `promoted` 非空才入队 organize（**条件链**）。

**organize（`organize.rs`）**：读 payload 指定原子 ∪ 历史 `scenario_id IS NULL` 的散原子（防漏，≤300）→ 连同 ≤100 个既有场景交 LLM → `create`（新场景）/`update`（并集合并 atom_refs + version+1）→ 场景批量 embed + 回填 `atom.scenario_id` → 仅 touched 非空入队 persona。

**persona（`persona.rs`）**：变动场景 + 各 aspect 当前画像交 LLM → 输出需更新的分面（**完整新版本内容**）→ 每分面 INSERT `max(version)+1`，`evidence_refs` 记录本批 scenario_ids 及其 atom_refs，`prompt_version` 落 `P_PERSONA.1`。aspect 白名单校验（7 枚举外丢弃）。

**consolidate（`consolidate.rs`）**：①近重复合并——余弦距离 <0.25 的 top-3 近邻聚类，去重防重复分组后交 LLM 判语义等价 → victims 置 archived、source_refs 并入 keep、keep hit_count+1；②stale 降权——90 天零命中且 confidence<0.9 的 ×0.7（下限 0.2）；③**自续期**：入队下周同任务（幂等键 `consolidate-{ISO周}`）。

### A3. 写入触发（`memory.rs::write_session`）

`distill=auto`：30s 防抖窗口（`trigger_auto_extract`，幂等键 `extract-debounce-{epoch/30}`，due=+30s）；`manual` 立即；`off` 不触发。

### A4. 检索与上下文包

- **`search`**：query 先 embed；L1/L2 各走单 SQL 混合检索（FTS jieba 预分词 + pgvector ANN + RRF=60 融合，`tsv_query_smart` 长查询 OR 兜底）；L3 体量小，token 包含过滤。无 embedding 通道自动退化纯 FTS。
- **`context_pack`**（AI 冷启动入口）：L3 全量 → L2 按 query 相关（无 query 取最近）→ L1 占预算余量；有 query 时 L1 走 `search_atoms` 语义相关（P0-R3 改造，替代旧 hit_count 排序）；字符预算硬裁剪，meta 带 truncated。

### A5. 治理面（API）

会话擦除（source_refs 标记 `erased:true` 保留结构）；原子手工增改（active↔archived 双向，superseded 禁手工）；画像回滚（以**新版本**落地历史内容，历史不可变）；`trigger_distill(full)` 手动触发。

### A6. 可信设计（现状里做得对的）

- 全链 job_events 留痕（每次 LLM 调用完整 I/O + prompt_version）
- 溯源三跳可回放：L3 evidence → L2 atom_refs → L1 source_refs → L0 原文
- 矛盾不是覆盖是 supersede 链（新→旧指针可追溯）
- LLM 输出全部白名单/长度/clamp 防护，防幻觉 id

---

## Part B：问题清单

按严重度排序。**P0=正确性/数据一致性，P1=质量/效率，P2=增强**。

### B1.【P0】extract 长会话无覆盖率保障——静默丢段

**位置**：`extract.rs::run_claimed`（全部轮次拼一个 `numbered` 文本，单次 LLM 调用）。
**问题**：一批会话全拼接进一个 prompt，会话多/长时超上下文，中间轮次被模型"看不见"且**无任何检测**——丢段静默发生。评论区 XBlueSky 教训原文：「长会话蒸馏需要覆盖率，而不是一个巨大的总结 prompt」。
**建议**：按字符预算切段（如每段 ≤8000 chars），逐段调用，每段要么产出 atoms 要么显式标记 `no_persistent_insight`，段索引记入 job_events。

### B2.【P0】arbitrate 近邻查询依赖候选自身 embedding——无 embedding 候选永远走"无相似"直通车

**位置**：`arbitrate.rs` 第 2 步（`ORDER BY embedding <=> (SELECT embedding FROM atoms WHERE id = $1)`）。
**问题**：extract 里 embed 失败的候选（`llm.embed` 出错直接 `?` 返回会重试，但**部分成功**时 `embeddings.get(i).cloned().unwrap_or_default()` 会写入**全零向量**）——零向量与其余零向量距离为 0，会把彼此误判为近邻；且对 `embedding IS NULL` 的手工原子完全不参与仲裁对照。另外子查询若候选 embedding 为 NULL，`<=> NULL` 结果为 NULL，**整行被过滤** → 候选被归入 no_similar 直接转正，跳过 LLM 仲裁。
**建议**：① extract 里 embed 部分失败时该原子 embedding 置 NULL 而非零向量；② arbitrate 近邻改为「候选向量非空才走向量，否则 FTS 相似（tsv @@ 或 ts_rank）兜底」；③ 零向量写入前校验维度和范数。

### B3.【P0】persona 证据链粒度错位——所有分面共享同一份全量 evidence

**位置**：`persona.rs` 第 100-109 行（evidence 查询在 aspects 循环内但 bind 的是**全部** `scenario_ids`，且 `SELECT atom_refs FROM scenarios WHERE id = ANY($1)` 对每个 aspect 重复执行同一查询）。
**问题**：①`[identity]` 分面的 evidence 和 `[skills]` 的完全一样——证据链失真，回滚/审计时无法回答"这个分面的结论基于哪些场景"；②同一查询在循环里重复 N 次（LLM 输出几个分面就查几次），纯浪费；③evidence 只到 scenario/atom id，没到 source_refs 的 session——溯源链在这里断一跳（要 JOIN atoms 才能到 L0）。
**建议**：让 LLM 在输出每个 aspect 时带 `evidence_scenarios`（它依据了哪些场景），按分面裁剪 evidence；查询提到循环外执行一次；evidence 组装时顺带 JOIN atoms.source_refs。

### B4.【P1】consolidate 聚类先验排除交叉组成员——近重复漏检

**位置**：`consolidate.rs` 第 37-45 行（`seen` 集合：组员被任何组收录后，后续组跳过）。
**问题**：A-B 近重复、B-C 近重复但 A-C 不够近时，B 被吸收进 A 组后 C 永远失去与 B 合并的机会。单用户百条量级影响有限，但语义上"防重复分组"防过头了。
**建议**：改为连通分量聚类（union-find 或递归扩展），或至少把 victim 只从"已裁决组"排除、候选仍可出现在多组。

### B5.【P1】arbitrate 的 LLM 调用不经 `chat_json_retrying`——JSON 抖动直接 job 失败

**位置**：`arbitrate.rs` 第 103 行（`llm.chat_json(...)` 裸调）；对照 organize/persona 用的是 `chat_json_retrying`。
**问题**：MiniMax 等模型长 JSON 输出偶发语法错（E2E 实测 wiki_generate 就撞过）。arbitrate 裸调意味着一次抖动 → JobError::Permanent → **整批仲裁失败**，且 extract 不会重跑（会话已 done），candidate 滞留到下次有新会话时才被兜底处理。consolidate 同样裸调（第 58 行）。
**建议**：两处改用 `chat_json_retrying`（就是 import 路径换个函数的事）。

### B6.【P1】duplicate 判定删候选丢溯源——被删内容蒸发

**位置**：`arbitrate.rs` 第 140 行（`DELETE FROM atoms WHERE id = $1 AND status='candidate'`）。
**问题**：判重即物理删除。若 LLM 误判（把互补信息判成 duplicate），候选内容**不可恢复**；且 L0 会话已 done，无法从源头重放这一条。与「superseded 保结构」的设计哲学不一致。
**建议**：改 `status='archived'` + `superseded_by=target`（复用 supersede 语义），保留审计与恢复能力；UI 的 active 过滤天然屏蔽。

### B7.【P1】persona 全量重写无 diff 审查——LLM 幻觉可静默污染画像

**位置**：`persona.rs`（prompt 要求"完整新版本"，代码原样落库）。
**问题**：prompt 说"场景里没有的信息不要写"，但无任何机械校验。LLM 幻觉一段"用户精通 Kubernetes"（实际没说过）→ 落库成 v(n+1) → **冷启动 context_pack 每次都注入这条幻觉**，直到人工发现回滚。幻觉进入最高杠杆层。
**建议**：低成本：persona 输出后做 content 与变动场景的关键词覆盖检查（新内容里的实词应能在 evidence 文本中找到出处），不达标降级为 needs_review 语义（落库但打标）；已有 `needs_review` 字段是 atoms 的，persona 可加同名列或先用 settings 记审核队列。

### B8.【P1】extract 防抖窗口固定 30s——活跃对话期间蒸馏欠时机

**位置**：`chain.rs::trigger_auto_extract`（`debounce_secs` 是 `MemoryService` 字段但硬编码 30，无配置入口）。
**问题**：对话越活跃，"等 30s 静默"越难达成——实际是每次写入都入队一个 30s 后的任务，窗口内新会话被**下一个**任务收走，行为接近"每 30s 批处理一次"而非防抖语义。注释宣称"同一 30s 窗口内的写入共用一个任务"依赖幂等键 `epoch/30` 恰好同桶，跨桶边界写入会连发两个任务。
**建议**：短期把窗口做成配置（env/settings）；长期若在意语义，改为「队尾已有 pending extract 时跳过入队，由该任务执行时自取全部 pending」（extract 本来就是全量认领，防抖键意义不大）。

### B9.【P2】L1 hit_count 只在 duplicate 时 +1——"使用热度"语义残缺

**位置**：`arbitrate.rs:146`（唯一 +1 点）、`consolidate.rs:108`（合并 +1）；检索路径（`hybrid.rs`）从不回写。
**问题**：字段名暗示"被命中次数"，实际是"被判重复次数"。stale 降权（consolidate ②）依据 `hit_count=0` 判"无人问津"——**必然大量误伤**：从未被重复过的正常原子全是 0，90 天后全体降权。这是 P0-R8（命中反馈）没做的直接后果，但 consolidate 已经在用它做决策了。
**建议**：①检索命中回写 `hit_count+1`（P1 清单 R8，低成本高价值）；②在那之前，把 stale 降权条件改成 `hit_count = 0 AND created_at < ...`（用 created_at 而非 updated_at，降权自身会刷新 updated_at 导致条件自锁——见 B10）。

### B10.【P2】stale 降权条件自锁——updated_at 被降权动作自身刷新

**位置**：`consolidate.rs` 第 120-123 行（`UPDATE ... SET confidence*0.7, updated_at=now() WHERE ... updated_at < now()-'90 days'`）。
**问题**：降权把 updated_at 刷成 now → 下一周该条不再满足 `updated_at < 90d` → 不会再降第二次。对单次降权恰好"歪打正着"，但若预期"持续衰减"则逻辑错误；且该条目一旦被 edit（updated_at 刷新）会重新进入降权视野，行为难以推理。
**建议**：降权不刷 updated_at（去掉 `updated_at=now()`，或改用 `created_at` 判龄 + 专门的 `last_decayed_at` 列）。

### B11.【P2】organize 无条件并入历史散原子——大表时 prompt 失控

**位置**：`organize.rs` 第 29-33 行（`id = ANY($1) OR scenario_id IS NULL`，LIMIT 300）。
**问题**：设计意图"防漏"，但 `scenario_id IS NULL` 的原子若长期无主题归属（LLM 判"孤立原子可以不处理"），**每次 organize 都重新进 prompt**——300 上限只是延缓。批量导入后前 N 次任务都会带上同一批"孤儿"。
**建议**：给"已尝试归组但未归入"打标（如 scenario_attempts 计数或 attempted_at），超过 2 次不再自动进 prompt，转 needs_review 队列人工处理。

### B12.【P2】persona 版本竞态——并发 run 时 max(version)+1 可撞号

**位置**：`persona.rs` 第 111-114 行（`SELECT MAX(version)+1` 与 INSERT 非原子；好在有 `idx_persona_aspect_version` 唯一索引兜底撞号报错）。
**问题**：单用户单 runner（concurrency=4 内同 kind 串行概率高）影响小，但 jobs 并发抢占下两个 distill_persona 同时跑会撞唯一索引 → 一个 JobError::Retryable 重试，浪费一次 LLM 调用。
**建议**：INSERT ... ON CONFLICT (aspect, version) 不适用（版本号要顺延），简单做法是包 advisory lock 或把 INSERT 改为 `INSERT ... SELECT ... FOR UPDATE`（锁 persona_aspects 该 aspect 行）；量级小，标记知晓即可。

---

## 附：问题↔既有 roadmap 映射

| 问题 | 对应 backend-enhancement 项 |
|---|---|
| B1 | R7（extract 长会话覆盖率）——P1 批 |
| B5 | 无对应（新增小修） |
| B6 | 无对应（新增小修） |
| B9 | R8（命中反馈）——P1 批 |
| B2/B3/B4/B7/B8/B10/B11/B12 | 新发现，建议并入 roadmap 再排期 |
