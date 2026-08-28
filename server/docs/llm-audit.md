# LLM 配置域实现审查

> 2026-08-28 · 与 [memory-audit.md](memory-audit.md) / [knowledge-audit.md](knowledge-audit.md) / [wiki-audit.md](wiki-audit.md) 同规格：Part A 机制全解，Part B 问题清单（每条含代码位置 / 影响 / 修法建议）。
> 审查范围：`crates/llm/src/` 全部 4 文件（provider / router / types / crypto）+ `crates/api/src/routes/llm_api.rs` + `crates/api/src/error.rs` 错误映射 + `migrations/0003_system_llm.sql` + master key 流（api main.rs / state.rs）。
> 关联 API key 签发/吊销段（同文件）一并覆盖；web UI 配置界面不在本审查范围（前端）。

## Part A 机制全解

### A1 数据模型（0003_system_llm.sql）

三表：`llm_providers`（name UNIQUE / base_url / api_key_encrypted bytea / models jsonb `[{"id","capabilities":[]}]` / is_default）；`settings` 表 `llm_routing` 键（`{purpose: [{provider, model}]}` flatten HashMap）；`llm_usage`（provider/model/purpose/tokens/latency/job_id/ts，按 ts 与 purpose 建索引）。

### A2 Provider 生命周期

- **创建**（llm_api.rs:64-99）：明文 key 经 KeyCipher 加密落库，响应只回 DTO（不含 key ✓）；**无任何输入校验**（见 L1）。
- **列表**（:103-131）：SELECT 不含 api_key_encrypted——密钥永不回显 ✓。
- **连通探针**（:142-255）：按 capabilities 挑第一个 chat 模型 + 第一个 embedding 模型；chat 发 1-token ping，embed 发「连通探测」（带 dimensions=1024，会触发 bge-m3 类的 dimensions 兼容重试）；双探针均 Ok 才记用量并报 ok=true。
- **没有更新 / 删除 / is_default 切换端点**（见 L2）。

### A3 密钥体系（crypto.rs）

AES-256-GCM：主密钥来自 `AGENT_MEMORY_MASTER_KEY`（64 hex），密文 = `nonce(12) || ciphertext||tag`，AAD 固定 `agent-memory-llm-key`。Debug 实现打码 ✓。**主密钥缺省回退**：main.rs 与 state.rs 在 env 未设时以 `"00"*32` 占位符构造 cipher（见 L10）——占位符可正常加解密，但与后续真实密钥互不兼容。

### A4 路由（router.rs + provider.rs resolve）

8 个 Purpose（extract/arbitrate/embed 低档；organize/consolidate/wiki_analysis 中档；persona/wiki_generation 高档）作路由键。解析三段（provider.rs:458-498）：

1. `settings.llm_routing` **每次现读**（配置即时生效，无缓存）；
2. 按 purpose 取有序回退链，逐条 `get(provider)`——**失败静默跳过**（无日志，见 L4）；
3. 链全 miss → 默认 provider（`is_default LIMIT 1`）+ 按 capabilities 匹配模型（embed 用途找 embedding 能力，chat 用途找非 embedding），**无匹配时 or_else(first) 兜底**（见 L1 陷阱）。

### A5 HTTP 客户端（provider.rs）

超时 chat 120s / embed 30s；熔断器按 provider name 全局共享（static CIRCUITS，跨 resolve 实例，5 次连续失败 Open 30s，HalfOpen 单试探）；429 Retry-After（delta-seconds + RFC 2822 HTTP-date）退避重试 ≤2；embed dimensions 参数被上游 4xx 拒绝时去参重试一次（bge-m3 实测路径）；错误分类 429/5xx/网络 → Transient，其余 → Permanent。

### A6 用量记账

`record_usage` 调用点（grep 全仓实证）：distill GatewayLlm 的 chat（llm_port.rs:176）与 embed（:228）+ test 探针（llm_api.rs:221/235）。**knowledge embed_job、knowledge/wiki/memory 检索的查询嵌入走 `registry.resolve → provider.embed` 直连，全部绕过记账**（见 L6）。

### A7 API 面（全部 require_admin——API key 持有者不可达 ✓）

POST/GET `/settings/llm/providers`、POST `/settings/llm/providers/{id}/test`、GET/PUT `/settings/llm/routing`、GET `/llm/usage`；API key 签发/列表/吊销（`amk_` 前缀、sha256 落库、明文仅创建响应出现一次、scopes 校验在 auth.rs）。

---

## Part B 问题清单

> 按严重度排序。**P0=配置正确性/密钥安全面，P1=健壮性/可运维性，P2=增强**。

### P0

**L1 · provider 创建零输入校验 + 错误分类误导 + 能力回退陷阱**
- 位置：llm_api.rs:64-99（name/base_url/api_key/models 均不校验直接 INSERT）；error.rs:116-120 + :64（重复 name 撞 UNIQUE → `From<sqlx::Error>` → **503 storage_unavailable + retryable=true**——把用户输入错误归类为可重试存储故障）；provider.rs:483-492（模型能力匹配 `.or_else(|| models.0.first())`——**embedding-only provider 被设为默认时，其第一个 embedding 模型会被选为 chat 模型**，全部蒸馏调用 400 且报错信息只说「HTTP 400」不指向配置根因）。
- 影响：空 name/空 key/`not-a-url`/`ftp://x` 全部入库；配错 name 重试得到「请稍后重试」的 503（单用户会以为服务坏了）；能力缺失的模型选择让「配了个只有嵌入模型的默认 provider」成为静默地雷。与 knowledge K 系列「入口校验」问题同类。
- 修法：① create 校验 name 非空且唯一（预查或捕获 23505 → 400）、base_url 以 http(s):// 开头且能 parse、api_key 非空、capabilities ∈ {chat, embedding}；② resolve 的 or_else(first) 改为按能力找不到直接 `NotConfigured("{purpose} 无可用模型，检查 provider capabilities")`；③ sqlx UNIQUE 冲突映射 400。

**L2 · provider 无更新/删除端点 + 密钥无版本化——配错只能改库**
- 位置：llm_api.rs 全文（仅 POST/GET/test；无 PUT `providers/{id}`、无 DELETE、无 is_default 切换）；crypto.rs（密文不含 key version 信息，无重加密路径）。
- 影响：key 填错/过期后**没有任何 API 途径修正**——只能 psql 直改；想下线一个 provider 同样无路（残留的 default provider 持续被 resolve 选中）；换 master key 后全部密文变砖且无批量重加密工具（与 L10 叠加成不可恢复陷阱）。前端 Settings 的「provider CRUD」实际只有 C 和 R。
- 修法：补 PUT（name 不可改，key 可选更新、models/is_default 可改）+ DELETE（校验非 default 或级联清理 routing 表引用）+ is_default 切换事务性降级旧默认；master key 轮换提供 `/settings/llm/providers/re-encrypt`（旧解密→新加密批量）。

### P1

**L3 · 多默认 provider 无约束——resolve 任意取一**
- 位置：llm_api.rs:76-87（INSERT 不查存量 is_default，可造出多行 true）；provider.rs:440-446（`WHERE is_default = true LIMIT 1` **无 ORDER BY**，多行时取哪行取决于物理顺序）。
- 影响：两次 create 都带 is_default=true → 之后默认路由结果不确定（表膨胀/VACUUM 后可能切换），且无任何告警。
- 修法：create 时 `UPDATE llm_providers SET is_default=false WHERE is_default` 同事务降级；resolve 的默认查询加 `ORDER BY created_at LIMIT 1` 兜底确定性。

**L4 · routing 表 PUT 零校验 + 幽灵 provider 静默跳过**
- 位置：llm_api.rs:273-281（任意 JSON 直接落库）；router.rs:23（`HashMap<String, Vec<RouteRule>>` flatten——purpose 键无枚举校验）；provider.rs:465-469（链中 provider 不存在时 `if let Ok(p) = self.get(...)` **静默跳过，无 warn 日志**）。
- 影响：purpose 键 typo（`extarct`）→ 该路由永不生效且无反馈；路由指向不存在的 provider 名（大小写/改名）→ 每次调用静默落回默认模型——用户以为在用 deepseek 实际用的默认，成本与效果双偏差；model 名不在 provider 的 models 列表 → 直到上游 404 才暴露。
- 修法：PUT 时校验（purpose 键 ∈ 8 枚举、每条 rule.provider 存在于 llm_providers、rule.model 在该 provider 的 models 中，违规 400 带明细）；resolve 链跳过时 `tracing::warn!`（幽灵路由可发现）。

**L6 · 用量记账覆盖不全——面板系统性低估**
- 位置：record_usage 调用点 grep 实证（A6）——knowledge embed_job（pipeline.rs 批量嵌入）、knowledge/wiki/memory 三域检索的查询嵌入（各自 service 的 `registry.resolve → provider.embed` 直连）全部不记账。
- 影响：知识库嵌入是最重的 token 消耗源之一，却完全不出现在 `/llm/usage`；检索查询嵌入按次数累积同样不可见——成本归因失真，无法回答「这个月 token 花在哪」。
- 修法：把 `resolve + provider.embed/chat + record_usage` 收进 ProviderRegistry 的门面方法（如 `registry.embed_for(purpose, req)`），调用方拿不到裸 provider——记账面从「约定」变「结构保证」。关联 R10（metrics）。

**L10 · master key 占位符陷阱——密文与后续真实密钥不兼容且无预警**
- 位置：main.rs:60-65（`cfg.master_key.unwrap_or("00".repeat(32))`）；state.rs registry()（同占位符回退）。
- 影响：服务首次无 env 启动 → 管理员照常建 provider（占位符加密，**无任何提示**）→ 之后部署补上真实 master key → 全部 api_key 解密失败（每次 LLM 调用报「解密失败（主密钥不匹配）」），且因 L2 无重加密路径，密钥只能重建。单用户自部署的高频踩坑序列。
- 修法：① 占位符生效时 create_provider 返回带 warning 字段（或直接拒绝建 provider，要求显式设 key）；② 启动日志显式告警「使用占位主密钥」；③ 与 L2 的 re-encrypt 路径配套。

### P2

**L5 · test_provider 幽灵代码 + 记账不对称**
- 位置：llm_api.rs:162-164（`registry.get(&name).await.map(|_| ()).err()` 结果整体丢弃——存在性检查写了等于没写，且误导读者以为用了解密结果）；:214-253（match 臂 `(Some(Err(e)), _)` 提前返回——chat 成功而 embed 失败时**连 chat 的用量也不记**，与「部分成功」语义不对称）。
- 修法：删幽灵行；每个探针独立记自己的用量。

**L7 · usage 端点固定 30 天窗口 + 500 条截断**
- 位置：llm_api.rs:299（`Utc::now() - Duration::days(30)` 硬编码）；provider.rs:521-532（LIMIT 500 无分页无总数）。
- 修法：`?days=` 参数 + 按日聚合端点（Dashboard 更需要聚合视图）。

**L8 · usage 端点为借 pool 构造整个 registry（占位 cipher）**
- 位置：llm_api.rs:292-296（`KeyCipher::from_hex_master(&"00".repeat(32))` 只为调用一个纯 SQL 方法）。
- 修法：usage_summary 挪成自由函数或直接在 handler 里写 SQL；至少去掉 registry 构造。

**L9 · 熔断器吞掉永久错误信号**
- 位置：provider.rs:288-293（非成功响应一律 `record_failure`——401/404 配置错误也计入）；:178-180（熔断打开返回 **Transient**「熔断器打开，快速失败」）。
- 影响：key 错误时前 5 次报「HTTP 401 Unauthorized」（Permanent，job 快速失败）→ 熔断打开后改报「熔断器打开」（Transient，job 退避重试到耗尽才 dead）——真正的配置根因被掩盖在最容易诊断的窗口之后。
- 修法：仅 Transient 类失败计入熔断；熔断打开的错误保持 Transient 但 message 附上「（此前连续失败 N 次，首因 HTTP 401）」。

**L15 · test 探针 max_tokens=1 对部分模型误报**
- 位置：llm_api.rs:194（固定 `max_tokens: Some(1)`）。
- 影响：reasoning 类模型（o1 系/部分深度思考模型）拒绝 max_tokens 参数或需要 `max_completion_tokens` → 探针失败 → 「provider 不可用」误报。
- 修法：探针 4xx 且错误信息含 max_tokens 时去参重试一次（复用 dimensions 兼容的模式）。

---

## 与既有系列的关联矩阵

| 本清单 | 对应 | 关系 |
|---|---|---|
| L1 | K 系列「入口校验」族（K5/K10 的模式） | 同类：写入口零校验让错误延迟到运行时爆发 |
| L2 | R 系列运维项 | provider 生命周期管理是 P1 工程健壮性方向的自然延伸 |
| L4 | K1/W1「静默回退」族 | 幽灵路由静默落默认 = 回退旁路家族在配置域的对应物 |
| L6 | R10（metrics） | 记账面收口是 metrics 的前置 |
| L9 | R5（已修：熔断/Retry-After） | R5 引入的熔断器与本条的永久错误掩盖互为副作用——修法需一并回归 |
| L3/L5/L7/L8/L10/L15 | 新发现 | 配置域特有 |

修复优先级建议：**L1+L3（创建路径校验与默认唯一性，半天级）→ L4（routing 校验 + warn）→ L2（U/D 端点 + re-encrypt）→ L6（记账门面）→ L10 → P2 按需**。L1/L2/L4/L10 同属「配置正确性」，适合一个 goal 连续修。
