# code-review 全量审查报告（2026-10-07）

> 七路并发（errors/logging/deadcode/aifriendly/coupling/bugs/config），引擎 codesleuth，LLM 档 review=GLM 5.3 flash。
> 审查范围 engram 当前工作区（HEAD 31dbd0e 含未提交改动），总耗时 233s，61 findings。
> 抽查验真：P1 幂等键（queue.rs 逐行核实✓）、AGENTS.md:83 迁移版本漂移（migrations_test.rs:18 核实✓）。
> 各路 findings 原始 JSON（含完整 statement/evidence）：本目录 `errors.json` 等 7 份 + `summary.json`。
> 本文件为分路摘要；任务映射见 [../roadmap.md](../roadmap.md)（T001-T008）与 [../open-questions.md](../open-questions.md)（Q001-Q008）。

## bugs（5 · high · 1×P1 + 2×P2 + 2×P3）

- **P1 幂等键终态复用断链**：`jobs/queue.rs:26-39` 幂等查询 `SELECT * FROM jobs WHERE idempotency_key=$1 LIMIT 1` 不过滤状态，注释却称「非终态或已成功才复用」——failed/dead 同键任务被原样返回，固定幂等键（如 wiki-generate-{source_id}）任务死后恢复链路静默中断。上游无防线：enqueue 是 20+ 调用方唯一收口。→ T001
- **P2 并发幂等键冲突抛错**：`queue.rs:28-64` 先 SELECT 后 INSERT 无原子性，并发同键第二个 INSERT 撞唯一约束被映射 `Permanent("idempotency_conflict")` 抛给调用方（转 503），与「命中返回既有任务」承诺矛盾。偶发、客户端重试可恢复。→ T002
- **P2 rerank 重复索引**：`core/unified.rs:311-323` 守卫只查长度与越界不查重复，LLM 返回重复索引时校验仍过，重复槽位第二次 take() 得 None，最高相关候选之一被静默降位——违背注释「异常一律降级原序」。解析侧 `234-243` filter_map 不去重。→ T003
- **P3 todos update 漏 title 长度校验**：create 查 `chars().count()>200`（core/todos.rs:195-197），update 只查空与 NUL（300-306），PATCH 可绕过创建侧约束。→ T004
- **P3（疑似）前端 BASE 三元两分支同为空串**：`web/src/lib/api.ts:6`——同源部署无差异，异域部署会全 404。→ T005

**已查无问题（死胡同备案）**：JobStatus::parse_filter 对空串报错视为有意设计，未列缺陷。

## errors（13 · high）

**正面结论**：API 统一错误体系健全（ApiError 枚举→(状态码,语义码,retryable,归因)，ErrorEnvelope 含 request_id，内部 Debug 只进日志、响应走 safe_message）；core EngramError 17 码注册表+未注册码 panic 强制纪律；jobs 恢复系统三态完整（退避重排/Dead 可 revive/永久）+幂等键+SKIP LOCKED+僵尸回收；LLM 外调有熔断+429 Retry-After 退避；吞错均带理由注释；生产 panic!/unwrap 零命中，expect 带论证；前端 ApiError+401 广播完整。

**弱点（归 P006 Deferred，本线不立项）**：统一模型未下沉（storage StoreError 仅 String 变体，roadmap 明写 Deferred）；传播链压平——`api/error.rs:143-156` JobError/LlmError 全变体映射 Unavailable 丢 BudgetExceeded/Permanent/Transient 语义；MCP 面 mcp_err data 恒 None；前端 localStorage JSON 空 catch 静默降级。

## logging（10 · high）

**正面结论**：集中初始化时序治理明确；PgLogLayer 经 mpsc(8192) 批量落 PG、channel 满即丢不反压（有意取舍）；request_id 全链路贯穿+响应头回带；错误与日志衔接一体化（internal_bug 自动 alert=true）；credentials.put 审计级结构化字段；生产代码零裸输出。

**弱点**：①secrets 防护靠纪律非机制（→Q007）；②4xx 统一打 ERROR 混入告警面（→T008）；③channel 满即丢+INSERT 失败仅 warn = 日志丢失盲区（已知取舍）；旧文档 `docs/plantree/005`「无落库」描述脱节（随手修）。

## deadcode（6 · medium）

确定死×3 → T006：①`scripts/quality/split_domain_modules.py`+`split_engine_modules.py`+`split_mcp_lib.py`（wave-2 治理残留，目标已消失，全仓 0 引用）；②`scripts/zztest_m10.py`（黑盒测试载具，已被 scripts/e2e/ 体系取代，债务清单两次登记待处置）；③`wiki-engine/insights.rs:319-327` ts_now()/uuid7() 带 allow(dead_code) 零引用。

文档漂移 → T007：`baseline/risk-hotspots.md:9` backup 兜底空卷条目已过时（backup.sh P001-T001 已改报错退出，engram_appdata 全仓仅剩文档一处）。

**防误杀备案（确证不删）**：`mcp/lib.rs:17,50` 的 allow(dead_code)/allow(unused_imports) 是 rmcp 宏注册所致，静态分析看不到引用——不可清理。`backup.sh:12-13` 硬编码 fallback DSN 属机器特定债务，登记备查非死代码。

## aifriendly（12 · high）

**正面结论**：AGENTS.md 四要素齐备且命令与 CI 逐字一致；验证闭环完备（CI 三 job+e2e 工作流）；MCP golden 快照=测试即规格典范；依赖方向单向无环；「为什么注释」文化浓厚（EN-47/expect 分类/攻击模型）；生态惯例布局良好。

**漂移/弱点** → T007 + Q008：README.md:131 死链 deploy/README.md（grep 全仓确认不存在）；AGENTS.md:83 迁移版本 63 vs 实际 75（漂 12 个版本）；mcp Cargo.toml:3「九域」vs README:89-93「十二域/13 工具位」（疑似，修前比对 DOMAIN_TOOLS）；「文档即数据」UUID 清单对离线代理是死引用，本地 plantree 仅 AGENTS.md:77 一笔带过。跨 crate 双写常量 ATOM_MAX_CHARS=500 → Q006。

## coupling（4 · medium）

总评：中等（局部纠缠，主干健康）。AppState 显式轻量 DI 容器、主干数据耦合健康。

→ Q004/Q005：DOMAIN_TOOLS 12 域名硬编码三方手工同步（dispatch.rs:204-227/264-268），新增域至少改 4 处；guard.rs:42-115 七连 require_* 逐字复制；state.rs:53-90 master_key→cipher 推导三连复制；jobs runner 把「workflow 类必须串行」写死在内部（注释自认）。

**死胡同**：只读工具无 VCS 历史能力，变更放大从代码内同步点间接推断非确证；跨端特性漂移仅看了 App.tsx 路由表，疑似。

## config（11 · high）

**正面结论**：必填压到最少（仅 DATABASE_URL），零配置可跑；.env.example 全注释态+生成命令；错误信封/MCP scope 提示/cli_fix_hint 三件套齐备（AI 可自纠）；三级渐进发现；外部调用默认带超时、MCP 白名单默认 loopback（缺省落安全侧）；启动回显贴心。

**弱点** → Q001/Q002/Q003：散点 env 现场读取（6 处入口无总表）；EMBEDDING_DIMENSIONS 死参数（非 1024 拒启）；engramctl 弱默认注入。轻微不一致：--token 与两个 env token 双通道。
