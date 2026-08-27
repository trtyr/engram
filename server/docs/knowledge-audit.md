# Knowledge 域实现审查

> 2026-08-27 · 与 [memory-audit.md](memory-audit.md) 同规格：Part A 机制全解，Part B 问题清单（每条含代码位置 / 影响 / 修法建议）。
> 审查范围：`crates/core/src/knowledge/`（mod / pipeline / chunking / ssrf）+ `crates/api/src/routes/knowledge_api.rs` + `crates/parsing/` + `migrations/0006_knowledge.sql` + 横切（tokenize / hybrid）。

## Part A 机制全解

### A1 数据模型与状态机

`documents`（0006_knowledge.sql）：`sha256 UNIQUE`（幂等键）、`status` 六态机 `pending → parsing → chunking → embedding → ready | failed`、`error` 文本、`raw_path`（上传文件/URL 快照落盘路径）。`chunks`：`UNIQUE(document_id, seq)`、`content`、`tsv`（GIN）、`embedding vector(1024)`（HNSW cosine）、`embed_failed` 标志，`ON DELETE CASCADE` 随文档删除。

### A2 摄取链（三步 job 链）

```
submit → enqueue_ingest（sha 幂等 + 落盘 + 入队）
  parse_document  → 抓取(SSRF)/读文件 → parse_bytes → extracted.txt → 入队 chunk
  chunk_document  → 结构感知切块 → 逐块 INSERT(tsv) → 入队 embed
  embed_document  → 64/批嵌入 → 失败块标 embed_failed(降级FTS) → ready + 清理临时文件
```

- **幂等键**：上传 = sha256(name + content_type + content)（pipeline.rs:27-39）；URL = sha256(url 串)（pipeline.rs:40-45）。命中即返回既有文档，API 层转 200（knowledge_api.rs:54-58）。
- **抓取**：URL 文档延迟到 parse 步抓取（`safe_fetch`，20MB / 30s 上限），成功后快照落盘 `{id}_url.html` 并回填 title（HTML `<title>` 提取，无则用 URL，pipeline.rs:186-201）；失败标文档 failed + job Permanent。
- **解析**：`spawn_blocking(parse_bytes)` 防 CPU 阻塞（pipeline.rs:236）；格式推断 `detect_format`：`.pdf/.docx/.html` 按扩展名，Content-Type 兜底，**其余一律默认按文本/markdown**（parsing lib.rs:16-39）。HTML 只取 body 内非 script/style 元素的直接文本子节点。
- **中间文件**：解析产物写 `{data}/uploads/{id}.extracted.txt` 供 chunk 步消费，embed 完成后删除（pipeline.rs:249-252, 438）。

### A3 切块算法（chunking.rs）

结构感知两路：markdown（含 `#` 行或 ` ``` `）按 `#/##/###` 标题切节（标题随节走）；纯文本按空行段落打包。超长节硬切（TARGET=800 字符/片）后贪心合并至 ≤1.5×TARGET，前进时保留一片重叠。参数：TARGET=800 / MAX=1400 / OVERLAP=1/7 ≈ 15%（D0009）。**注意：UTF-8 字节长度计算，中文实际每块 ≈450-700 字**。

### A4 SSRF 防护（ssrf.rs）

五层：① scheme 白名单 http/https；② 私网全拒（IPv4: loopback/private/link-local/CGNAT/组播/保留/TEST-NET/198.18，IPv6: ULA/链路本地/组播/v4-mapped 递归复检）；③ 重定向手动逐跳（≤4 跳）每跳重新校验；④ DNS pinning——解析后只对已校验 IP 建 client（防 rebinding），有 v4 时仅用 v4；⑤ Content-Length 预检 + 流式读双重大小封顶。**例外：检测到 `HTTPS_PROXY` 等代理环境变量时，跳过 ③④ 的地址校验（ssrf.rs:92-96, 110-114），注释声明「信任边移到代理」**——见 K3。

### A5 检索（mod.rs search）

单条 SQL：FTS（`to_tsquery('simple', tsv_query_smart(query,3))`，rank=ts_rank）+ 向量 ANN（`embedding <=>`，仅 has_vec 时）双候选集各 LIMIT 200，RRF 融合 `1/(60+rank)`，带文档标题，snippet 取前 200 字符。查询嵌入失败或无 provider → 纯 FTS。走 R2 的 smart tsquery（≤3 token AND，>3 OR）。

### A6 删除

DB 行删除（chunks 级联）→ 删原始文件（raw_path）→ 删 extracted.txt（mod.rs:119-141）。不可恢复（无回收站）。

### A7 API 面（7 端点，knowledge scope）

POST /knowledge/documents（URL）、POST /knowledge/upload（multipart，50MB 上限）、GET documents（status 过滤 + 游标，limit≤200）、GET documents/{id}、GET documents/{id}/chunks（**硬编码 500 上限**，内容截 300 字）、DELETE documents/{id}、POST /knowledge/search（max_items≤100）。

---

## Part B 问题清单

> 按严重度排序。**P0=正确性/数据一致性/安全，P1=质量/健壮性，P2=增强**。
> 与 memory-audit B 系列同类的问题标注了对应关系。

### P0

**K1 · sha 幂等墙无状态过滤 + 失败全归 Permanent——一次网络抖动永久卡死该 URL/文件**
- 位置：pipeline.rs:49-58（`SELECT id, status FROM documents WHERE sha256=$1` 无 status 过滤，注释明写「任何状态」）+ pipeline.rs:157-160（`fail()` 把一切失败包成 `JobError::Permanent`）+ ssrf FetchError 的 Network/Dns 变体同样走 Permanent。
- 影响：URL 抓取一次瞬时超时 → 文档 failed + job failed；用户重新提交同一 URL → 命中幂等墙返回既有 failed 文档（200），**不会重新入队**。且 job 级幂等键 `ingest-{id}`（pipeline.rs:105）构成第二道墙——即使绕过文档级检查，revive 前同名键也入不了新 job。唯一出路是 DELETE 再重传（用户不可发现）。与 wiki 的 sha 幂等墙（d333 观测）同构，且知识域连「变文本换 sha」的自救都没有（URL 内容变了 sha 不变）。
- 修法：① 文档级——sha 命中且 `status='failed'` 时重新入队（键改 `ingest-{id}-{attempts}` 或 revive 原 job）；② 错误级——`FetchError::Network/Timeout` 归 Retryable（走队列退避重试），仅 `PrivateAddress/Scheme/TooLarge/Dns` 保持 Permanent；③ parse 失败重试时 URL 快照已落盘的走本地路径（代码已有此意，pipeline.rs:187 注释「重试不重复抓」）。

**K2 · enqueue 结果被 `.ok()` 吞掉——文档永卡 pending**
- 位置：pipeline.rs:101-108（`queue.enqueue(...).await.ok();` 整个 Result 丢弃）。
- 影响：documents INSERT 成功后 job 入队瞬时失败（DB 抖动）→ 无 job、无错误、无重试，文档永远停在 pending。用户看到的是「提交成功但永远不 ready」，且无任何事件可查。
- 修法：入队失败时回滚文档行（DELETE）返回 Storage 错误让上层重试；或至少把失败冒泡为 `KnowledgeError`，配合对「pending 超时无 job」的清扫。

**K3 · SSRF 代理旁路——设了 HTTPS_PROXY 私网校验全跳过**
- 位置：ssrf.rs:92-96（via_proxy 检测）+ ssrf.rs:110-114（代理模式 `addrs=vec![]`，跳过 DNS 校验与 pinning）。
- 影响：本机代理（如 Clash 127.0.0.1:12543）本身能直连私网——服务进程若带代理环境变量运行（上海网络环境很常见），任何持有 knowledge scope 的 API key 持有者可提交 `http://127.0.0.1:8080/...`（agent-memory 自己的管理面）、`http://169.254.169.254/`（云元数据）等 URL，响应体经代理原样返回。整套私网黑名单形同虚设。
- 修法：代理模式下仍应对原始 URL 做本地解析 + 私网校验（拦字面量与常规解析结果；代理端 DNS rebinding 是残余风险，需文档声明）；或提供 `AGENT_MEMORY_FETCH_*` 独立代理配置与 `ALLOW_PRIVATE_FETCH` 显式开关，默认拒绝。

**K4 · embed 响应短于批次 → NULL 向量 + `embed_failed=false` 双重静默**
- 位置：pipeline.rs:383-392（`resp.embeddings.get(i)` 得 None 时 bind NULL，且 `SET embedding=$2, embed_failed=false`）。
- 影响：网关返回 embeddings 数量少于 inputs（异常但真实存在，尤其中间层 newapi 类）→ 缺失块写入 NULL 向量**同时清除降级标志**：向量检索不可见、FTS 无异常提示、chunks API 显示 embed_failed=false——三处都说「正常」。与 memory 域 B2（零向量旁路）同类但更隐蔽。
- 修法：`resp.embeddings.len() != batch.len()` 时整批按失败处理（标 embed_failed + warn 日志），或至少逐块 `embed.map(...).unwrap_or` 分支里对 None 标 embed_failed=true。

**K5 · 未知二进制格式默认按文本解析——mojibake 入库且可检索**
- 位置：parsing lib.rs:38（`detect_format` 兜底返回 `"md"`）+ lib.rs:51-57（`String::from_utf8_lossy` 把非法字节替换为 U+FFFD 后当正文）。
- 影响：上传 `.xlsx/.pptx/.epub/.zip` 等未支持格式 → 不报错，二进制垃圾经 lossy 转换变成乱码文本，切块、嵌入、入库、可被搜到——污染向量空间与 FTS 索引，用户毫无感知。
- 修法：默认分支改为「二进制嗅探」——含 NUL 字节或 UTF-8 校验失败即 `ParseError::Unsupported`；仅对 `.txt/.md` 及有效 UTF-8 文本走默认路径。

### P1

**K6 · 并发同 sha 提交竞态 → 唯一约束冲突 503**
- 位置：pipeline.rs:50-99（check-then-insert，无事务无 ON CONFLICT）。
- 影响：双击上传/客户端重试并发到达 → 两个 SELECT 都未命中 → 第二个 INSERT 撞 `sha256 UNIQUE` → `KnowledgeError::Storage` → API 503（正确语义应是 200 幂等命中）。对照：jobs 队列对同一场景显式定义了 `idempotency_conflict`。
- 修法：`INSERT ... ON CONFLICT (sha256) DO NOTHING RETURNING id`，无返回行再 SELECT 既有——单往返无竞态。

**K8 · embed 批次失败永久降级，无恢复入口**
- 位置：pipeline.rs:402-424（Err 分支整批标 embed_failed，不区分 Transient/Permanent，不重试）。
- 影响：上游嵌入服务宕机 5 分钟期间摄入的文档 → 全部 FTS-only，服务恢复后**没有任何机制补嵌**。唯一恢复路径是 admin 手工 POST /jobs/{id}/revive 重跑 embed job（可行但完全不可发现，且 revive 是通用运维操作）。此外 embed 无 token/成本记账（chat 侧有 400k 预算，embed 侧裸跑）。
- 修法：① 失败分类——Transient 错误让 job 整体 Retryable（下轮重跑会对 embed_failed=true 的块重嵌？不会，当前实现重嵌所有块——顺带修为只嵌 `embedding IS NULL OR embed_failed`）；② 提供 `POST /knowledge/documents/{id}/re-embed` 语义端点（内部即 re-enqueue embed job，只处理缺失块）。

**K9 · URL 瞬态网络错误与永久错误同归 Permanent（K1 的错误分类半部，单列以便分期修）**
- 位置：pipeline.rs:202-206（`Err(e) => { let m = format!("URL 抓取失败: {e}"); ... return Err(fail(m)) }`）——`FetchError` 已有精细变体（Scheme/PrivateAddress/TooManyRedirects/TooLarge/Network/Dns），但 fail() 一律 Permanent。
- 影响：DNS 抖动、对端 502、TLS 瞬断都一次定罪。队列的退避重试机制（200ms*2^n 封顶 30s）对这个最需要重试的 IO 场景完全没用到。
- 修法：见 K1②。SSRF 判定类保持 Permanent 防恶意 URL 反复打探测。

### P2

**K7 · 空 token 查询静默零召回——单字/纯标点查询无声返回空（三域共用）**
- 位置：tokenize.rs:21-33（`keep()` 过滤单汉字与 1 字符 ASCII）→ `tsv_query_smart` 返回空串 → `to_tsquery('simple','')` 产生**空 tsquery**（PG 16 实测：NOTICE「doesn't contain lexemes」而非错误，不匹配任何行）。调用点：knowledge mod.rs:160-164、search/hybrid.rs:33-35 与 88-90（memory atoms/scenarios）、wiki-engine service.rs。
- 影响：查询「书」「的」或纯标点 → 各域 search **静默返回空结果**，无错误无提示。与索引侧过滤语义一致（单字本就不进索引），故不是崩溃而是可用性缺口——用户不知道为何搜不到，也无反馈引导换词。
- 修法：各调用点判空短路：tokenize 后 tokens 为空直接返回 `vec![]`（可选返回 BadRequest「查询词太短」提示），在 tokenize.rs 提供空判辅助一处修三域。**注意不要用哨兵串**——实测 `to_tsquery('simple','!')` 报 `no operand in tsquery` 错误，哨兵会把静默缺口变成真 500。

**K10 · 游标分页 created_at 非唯一——同刻多行跨页丢失/重复**
- 位置：knowledge mod.rs:81-84（`created_at < $2 ORDER BY created_at DESC`）；memory.rs:186/251 同款。
- 影响：timestamptz 微秒精度，批量摄入同一事务内多行同刻 → 翻页跳行或重复。单用户规模概率低。
- 修法：`(created_at, id)` 元组游标：`WHERE (created_at, id) < ($2, $3)`。

**K11 · chunk 重跑残留旧块——ON CONFLICT 只 UPDATE 不 DELETE**
- 位置：pipeline.rs:313-327（`ON CONFLICT (document_id, seq) DO UPDATE`）。
- 影响：chunk job 被 revive 重跑且新切块数少于旧数（重解析后文本变短）→ 高 seq 旧块残留，新旧混合。触发面窄（需 job revive + 内容变化）。
- 修法：chunk 写入前 `DELETE FROM chunks WHERE document_id=$1`（本 job 全量重建语义），或按 max(seq) 截断。

**K12 · extracted.txt 中间文件泄漏——异常路径无清扫**
- 位置：pipeline.rs:438（仅 embed 成功路径删除）；无孤儿扫描。
- 影响：embed job 死亡（重试耗尽）、进程崩溃、文档被删但 job 在飞等路径下 `{id}.extracted.txt` 永久残留，data 目录缓慢膨胀。单用户长年运行可观。
- 修法：启动时清扫「无对应 pending/running job 的 extracted.txt」；或复用定期维护 job（consolidate 同款机制）。

**K13 · search 查询嵌入失败零日志静默降级**
- 位置：knowledge mod.rs:145-156（`.ok()` 吞掉 embed 错误，无 tracing）。
- 影响：嵌入 provider 配置坏了 → 所有查询静默退化为纯 FTS，无任何可观测信号，召回率下降无人知晓。
- 修法：失败分支加 `tracing::warn!`（每 N 次采样防刷屏）；可观测性长期项挂 R10 metrics。

**K14 · chunks API 硬编码 500 上限截断且无总数**
- 位置：knowledge_api.rs:162（`svc.chunks(id, 500)`），mod.rs:103-116 无 COUNT。
- 影响：超大文档（>500 块，约 >70 万字符）预览静默截断，前端无从得知不完整。
- 修法：返回 `{total, chunks}` 结构 + seq 游标分页。

**K15 · URL 内容级去重缺失**
- 位置：pipeline.rs:40-45（URL 幂等键 = sha256(url 串)）。
- 影响：同一页面带不同 query（utm 参数）/http-https/尾斜杠 → 多份内容相同的文档各自切块嵌入，检索结果重复占位。
- 修法：抓取后对正文 sha 二次判重（命中则复用既有文档或标记变体）；URL 规范化（去 utm_*、排序 query）作为廉价第一步。

**K16 · 大文件无切块数/嵌入成本护栏**
- 位置：mod.rs:67-71（仅 50MB 字节上限）；chunk/embed 无数量上限。
- 影响：50MB 文本 → ~3.6 万块 → ~570 批嵌入调用（1024 维）——一次误传跑数小时管道 + 显著 provider 费用，无确认无预算闸。
- 修法：submit 时预估 chunk 数超阈值（如 2000）要求显式 `confirm=true`；embed 侧接入用量记账（对应 R15 大文件护栏可合并实现）。

---

## 与既有 roadmap 的关系

| 本清单 | 对应既有条目 | 关系 |
|---|---|---|
| K7 | R2（tsquery 召回） | 同一文件的延续：R2 修了长查询 OR 兜底，K7 是空 token 补刀 |
| K16 | R15（大文件护栏） | 合并实现 |
| K13 | R10（metrics） | 挂载点 |
| K4 | memory B2（已修） | 同类问题在知识域的对应物，修法可参照 |
| K1 | wiki sha 幂等墙（E2E 观测） | 同构问题三域第二例，建议一并出通用方案 |

修复优先级建议：**K2+K6（提交路径正确性，半小时级）→ K1/K9（重试语义）→ K4/K5（静默降级）→ K3（代理 SSRF，看部署形态定急缓）→ P2 按需（含 K7 空 token 静默零召回，一行修三域）**。
