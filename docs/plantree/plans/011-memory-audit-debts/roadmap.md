# P011 Roadmap

> 任务身份/状态/顺序的唯一权威。全部 Planning——每条需拍板后进 In Progress。
> 每条带实读证据（文件:行号，2026-10-03 实读）。

## Done

- [x] **T012 · JEV 决策模型适配评估**（2026-10-03）——两轮中文评测，结论可落地并经
  用户拍板（决策 001）：构造用例 10/10；生产回放 25 段一致率 84%（方差≤0.06）；
  arbitrate 三判 7/7；107 次调用 <$0.002。评测依据与阈值分档见
  [decisions/001-jev-l01-guard.md](decisions/001-jev-l01-guard.md)；完整评测数据在
  本文件 git 历史（42cc110..bf30073）。

## In Progress

（空）

## Next（待拍板，按建议优先级）

### 正确性

- [ ] **T001 · 审计事件不进 logs 流**【Q001】
  repo/memory/ops.rs:88 `audit()` 直插 jobs 表伪造 `status='succeeded'` 的 job 行
  （注释自认「写一条已完成的 job 行」）。调用点 11+：session_void_cascade /
  session_erase_cascade / session_unvoid_restore / correct_atom / edit_atom /
  review_confirm / review_discard / atom_archive / kv_delete / entity_merge /
  edit_persona / delete_entity。**与 P010「日志是唯一时间线」原则冲突**——审计只在
  MCP jobs 域可见，logs 流与前端日志页看不到。
- [ ] **T002 · mark_superseded 双 active**
  distill/arbitrate.rs:296 `WHERE id=$2 AND status='active'`——旧条已 archived 时
  UPDATE 0 行但候选已转正（:235 先 promote 再 supersede），同主题双 active。
  蒸馏链篇坑清单第 5 条实证。
- [ ] **T003 · 场景成员双轨漂移**
  写侧：distill/organize.rs:270 `update_scenario` 的 atom_refs 是**并集**（旧∪新，
  只进不出）；:317 `refresh_embeddings` 的 atoms.scenario_id 是**覆盖回填**。
  原子被场景 B update 拉走时：A.atom_refs 残留 + scenario_id 改指 B。
  读侧分裂：entity_scenarios（repo/entity.rs:67）/timeline 用 scenario_id 正查，
  converge（scenario_converge.rs:75）用 atom_refs 反查——两轴一致性仅靠 converge 兜底。
- [ ] **T004 · update_atom_full 单值字段清不掉**
  repo/memory/atoms.rs:271-274 superseded_by/occurred_at/valid_until/sensitive 全
  `COALESCE($X, 保留旧值)`——传 None 永远无法置回 NULL。EN-BUG-1（study learned_at）
  同族盲 CASE，方向相反：那边保不住、这边清不掉。
- [ ] **T005 · 实体子串归并误吞 + 关系 lookup 口径分裂**
  distill/extract.rs:226 `position(lower($1) in lower(name))>0 OR 反向` 双向包含
  + `LIMIT 1` 无 ORDER BY——「云」可吞「星云」；:298 关系 lookup 用 `name=$1` 精确
  匹配，与挂链的子串归并口径不一致。坑清单第 3 条实证（P001 Deferred 只挂了
  ORDER BY 半条，双向包含与口径分裂在此线补全）。

### 性能

- [ ] **T006 · extract claim 全量抢占无 LIMIT**
  distill/extract.rs:46 `UPDATE raw_sessions SET distill_status='processing'
  WHERE distill_status='pending' AND ... RETURNING ...`——一次认领全部 pending，
  无 LIMIT。会话量级增长后第一个慢查询（坑清单第 2 条实证）。
- [ ] **T007 · LLM 无熔断**
  distill/llm_port.rs budget_tokens 声明未用——蒸馏风暴无成本闸门（坑清单第 4 条）。

### 卫生

- [ ] **T008 · revive_entity 死代码 + 永不复活后果**【Q002】
  repo/memory/entity.rs:459 全仓零调用；extract.rs:264 注释明示「复活语义已废除」。
  实际后果：孤儿实体归档后即使同名再次出现也**新建实体**（link_entity 只查活体），
  旧档案（summary/revision）永沉归档态。删函数或恢复接线，二选一。
- [ ] **T009 · stale 注释三处（决策 001 后未跟上）**
  ① search/hybrid.rs:86「P3：sensitive 原子默认排除」——主检索路径已全传 true
  （core/search.rs:250/472/332）；② core/memory/search.rs:426 F4 注释「归档或标敏感
  →收敛」——实际仅归档触发（:377，标敏感不触发是 P003-T001 决策）；③
  repo/memory/atoms.rs:368 recent_active_atoms 注释「过滤过期与敏感」——SQL 只滤过期。
- [ ] **T010 · mcp_test.rs:446 文案滞后**
  报错文案「应为十二个域工具」vs 断言 `tools.len()==13`（P010 加 logs 域后没跟）。

### 已拍板（决策 001 · JEV 上 L0→L1，2026-10-03）

- [ ] **T013 · 决策模型接入面 + JEV 配置**
  ① DistillLlm trait 增 `decide` 原语（或独立 DecideLlm port）+ OpenRouter Decisions
  API 客户端（POST /api/alpha/decisions，typesafe/jev-1.13）；② 配置面：Web 设置
  「AI 功能」新增 JEV 区块——**接口仅支持 OpenRouter**（无 provider 选择器，默认且唯一），
  用户填 API key（加密存储，倾向复用 llm_providers 体系加 decide capability 以白嫖
  KeyCipher/re-encrypt/测试链；备选 settings 单行 JSON），enabled 开关 + 模型名 +
  阈值参数（reject/review 两线）；③ HTTP GET/PUT /settings/jev（Admin）。验证：配置
  改动即时生效 + 门禁。
- [ ] **T014 · L0→L1 保险级联落地（含归因回执）**
  extract claim 后、chat 精抽前逐段 JEV 判定：noul 概率 `<0.3` 跳过（事件流记
  「JEV 拒绝 p=xx」）/`0.3~0.5` 照常精抽但产物提级 needs_review/`≥0.5` 放行；
  归因回执用 choice 五分类（user_facts/project_internal/transient/learning/chitchat）
  同请求产出拒绝理由（原 T011 方案并入此处，抽取率可统计）。JEV 失败/未配置 = 降级
  直通精抽（不阻塞蒸馏主链）。验证：logging_test 顺序壳加回放用例 + 抽取率事件可查。
- [ ] **T015 · 阶段二：arbitrate choice 化 + consolidate noul 化**（依赖 T013/T014 落地）
  arbitrate：每候选 choice{new,duplicate,contradicts} 替换 prompt-and-parse，低置信
  进待审（顺治漏判兜底问题）；target 指认取 top1 相似（最简方案，设计期可复审）。
  consolidate：近重复「语义等价吗」noul 化。评测支撑：三判 7/7（T012 评测记录）。
- [ ] **T016 · 蒸馏链串行保证写死（结构不变量，非配置项）**
  生产实锤（2026-10-03）：近 7 天 extract_atoms 12 dead / 17 succeeded；蒸馏链无串行
  保证——organize 双跑抢占同一批未归组原子（重复场景风险）、persona 双跑撞
  UNIQUE(aspect,version)。
  **设计原则（用户拍板 2026-10-03）**：串行性是蒸馏链的**正确性不变量**，必须写死在
  代码里作为结构性保证（类似消息队列分区串行语义），**不能是 env 可调项**——env
  能调的是性能参数，不是「会不会产生重复场景」。现有三层防护（claim 状态机互斥 /
  防抖幂等键 / 可选 per-kind env）是补丁拼盘，缺结构性保证。
  落地方向：① runner 层引入**工作流类别**——蒸馏链 kind（extract_atoms/arbitrate_atoms/
  organize_scenarios/distill_persona/consolidate）标记为 workflow 类，全局同时只允许
  **一条活跃蒸馏链**（chain-level mutex：链头 extract 抢到 chain 锁，链尾释放），
  写死无 env；② 其余独立 kind（wiki_*/cg_*/embed 等维持并发池，per-kind cap 仅对
  非蒸馏 kind 开放）；③ LLM 熔断核查随本任务：坑 4「无熔断」部分过时（llm 层有
  熔断器，error 文案实证），budget 总闸仍缺，随 P012 T002 一并落地。
  用户原话：「这个东西应该写死的」「应该有一个类似于消息队列的一套机制，能够保证
  稳定，并且保证不会冲突」。
- [ ] **T017 · KV 准入 JEV 闸（写前把关）+ 存量大扫除**
  KV 写入路径（remember(mode=kv)/kv_put 全部入口）前置 JEV choice 判定，一次请求
  同时完成判定与归因：`kv_fit`（放行入库）/ `prefer_atoms` / `prefer_credentials` /
  `prefer_assets`（含 projects）——非 kv_fit 拒绝入库并在错误信息里建议去处，由
  调用方自行转投。准入判据写进 criteria 四条：**逐字必需**（精确值，LLM 转述会变形）/
  **非机密**（机密走 credentials）/ **非台账**（基础设施走 assets）/ **跟用户相关**。
  降级语义：JEV 未配置或调用失败 = **放行**（KV 写入是调用方显式动作，闸门把关不
  阻塞——与蒸馏侧「失败直通」同哲学）。配置复用 T013 的 JEV 配置面（同 key 同开关）。
  存量大扫除（2026-10-03 摸底共 9 条）：workrule_no_push_during_review → 转 atoms
  （偏好/约定）；agent-compose-beijing-hub → 转 assets；fleet-npm-registry-status →
  核实后删；其余 6 条（contact-*/resume-path/music-*）合规保留。
  动机：KV 正从「精确值登记簿」退化为「不知道放哪就放这」的杂物抽屉（9 条中 2 条
  放错）——用 JEV 把准入判据从纪律变成机制。
- [ ] **T018 · MCP memory 域动作面收敛 32→6（旧名删除，不做兼容）**
  用户拍板（2026-10-03）：System prompt 由用户自行维护，旧名无需兼容——直接删。
  目标形态（六动词全 mode 归并）：
  `remember`（atom/session/append/kv——吸收 write_session/append_session/kv_put）·
  `recall`（search/context/entities/kv_get/kv_search）·
  `browse`（atoms/sessions/session/kv/**scenarios/persona**——吸收 list_atoms/
  list_sessions/get_session/kv_list/scenarios_list/persona_get）·
  `revise`（correct/persona/**archive/merge**——吸收 correct/persona_edit/
  atom_archive/entity_merge）·
  `review`（distill/result/confirm/discard/**duplicates**——吸收 distill/distill_result/
  confirm/discard/atom_duplicates/entity_duplicates，duplicates 带 target 参数）·
  `forget`（void/erase/restore/**kv**——吸收三个会话动作+kv_delete）。
  mode 改名：session_append→append（去旧名影子）；kv_delete→forget(mode=kv)。
  语义全覆盖 32→6，零丢失；实体 CRUD 本就不在 MCP 面（HTTP/Web），不受影响。
  **动机（修正口径）**：MCP 每域一个工具位，tools/list 不膨胀；真实代价在 help
  渐进发现消化 32 条（25 条重复语义）、双份参数结构维护（RememberParams 与
  WriteSessionParams 并存）、动作选择错误率、管理台开关噪音——EN-235 改造的
  未竟终章。变更面：dispatch.rs 动作表与 handler 合并 + golden 快照重生成 +
  mcp_test 三处断言（照 P010 两轮门禁教训预防）+ Web 管理台跟随。验证：门禁 +
  golden + Agent 实测（pi 自身即消费者）。

## Deferred / 交叉引用

- **空产出段 JEV 补抽哨兵**：T014 落地后的可选玩法——对 0 原子段跑 JEV，高 P（疑似
  历史误杀）进二次精抽。前置：复核 2 条疑似误杀坐实后再立项
- prompt_version 归因列（atoms/scenarios）→ **P001 Deferred 已挂**，不重复开
- 实体归并 LIMIT 1 加 ORDER BY → P001 Deferred「杂项」已挂（本线 T005 补全另两半）
- 结构观察（非缺陷，登记备查）：敏感标记在 L2 丢失——scenarios 表无 sensitive 列，
  敏感原子进场景后场景层面无标记（决策 001「纯标记」口径下无害）
