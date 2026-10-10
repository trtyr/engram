# P019 Roadmap

> **M1 已收官（2026-10-09 晚，commit 02126f8）**：T001 五条 P1 全部落地 + 各配回归测试，门禁三件套全绿。
> **M2 已收官（2026-10-10 午）**：T002 鉴权族七条全部落地 + 各配回归测试，门禁三件套全绿。
> **M3 已收官（2026-10-10 午后）**：T003 前端可用性族八条全部落地 + 各配测试，web 四件套全绿（tsc/lint/test 105/build）。
> 迁移：migrate:ro 归 Q004 拍板（不在本批）。

来源：docs/review/ 38 篇检查项（2026-10-09 审查，errors/config 补跑后定稿），逐条发现以各篇 `features/<slug>.md` / `global/<dim>.md` 为详细依据（含 file:line 证据）。此处只登记任务级条目。

## T001 P1 正确性五条（先行）——✅ done（02126f8）

| # | 内容 | 依据 |
|---|---|---|
| 1 | distill extract 重试重放→原子重复落库：persist_atoms 与 mark_sessions_done 同事务或按会话幂等 | features/distill-crate.md #1 |
| 2 | cg-bridge run_cli 超时不杀子进程（kill_on_drop）+ register clone 同构；复核 Timeout=Retryable 放大效应 | features/cg-bridge-crate.md #1 |
| 3 | cg_index/cg_sync 默认 300s visibility_timeout < 任务耗时→僵尸回收双执行；长任务设大超时或心跳续期；complete() 加 running 态守卫 | features/jobs-crate.md #1 |
| 4 | wiki-engine rebuild_all_links 无 page_type 过滤→系统页边瘫痪 orphan lint/repair | features/wiki-engine-crate.md #1 |
| 5 | tickets 全量导出被 core list `limit.min(500)` 静默截断（total 仍报全量） | features/tickets-domain.md #1 |

## T002 鉴权/越权族——✅ done（M2）

- study:ro 全拒（require_study 对 ReadOnly 直接 Err）— features/auth-platform-domain.md #1
- /search 跨域越权：wiki-only key 可读 todos/tickets/entity（require_search 精确匹配绕开 domain_access）— features/scope-system-13.md #1 + features/llm-platform-domain.md #1
- original:ro 可执行 KV 写入 — features/mcp-crate.md #1
- llm/erase/cron 的 :ro 变体可签发但全域不可用 — features/scope-system-13.md #3
- migrate:ro 可 import（文档自认故意，待拍板口径）— features/scope-system-13.md #4
- tool_scope `_ => "memory"` 兜底→study/logs 工具发现面错位；删兜底改显式表 — features/mcp-crate.md #2 + global/coupling.md #2
- admin 登录把 DB 不可用计入爆破失败（5 次抖动锁 15 分钟；按错误类别豁免）— features/admin-pbkdf2-login.md #1
- MCP call_tool 的 action 非字符串时动作级检查整体跳过（疑似：遍历 handler 确认或强制走 unknown_action）— features/scope-system-13.md #2

## T003 前端可用性/正确性族——✅ done（M3）

- 幽灵接口：Memory.tsx 调已退役 POST /memory/distill（改 /memory/maintain）；同退役端点残留 verify 脚本与 e2e — global/fullstack.md #1/#2
- Wiki 保存/回滚后阅读区不刷新（重取 page 后 setOpen）— features/wiki-domain.md #1/#2
- Wiki folder 清空无法回根目录（三方契约不一致）— features/wiki-domain.md #3
- confirm.tsx settle 竞态（绑定闭包 Request 实例）— features/api-crate.md #1
- Study 热力图 fallback 崩溃（learned_at ?? '' → RangeError + 索引错位）— features/study-domain.md #1
- Credentials DangerZone 删除失败仍报成功 — features/credentials-domain.md #4
- Todos 搜索防抖；Tickets >200 条前端不可见（total 未用）；Tickets >500 导出截断归 T001-5 — global/frontend.md #6 + features/tickets-domain.md #3
- App 无 ErrorBoundary / Shell 无 catch-all / /file-view 探活竞态 — features/lazy-route-pages-17.md #2/#3/#4

## T004 数据保真族（storage/transfer + 域写路径）

- import_atom 补 strength/source_kind 两列 — features/storage-crate.md #1（已确证）
- import_session/import_kv_entries 缺字段绑 NULL → NOT NULL 中断（对齐 import_entity 的兜底模式）— features/storage-crate.md #2
- import_wiki_promotions library_id 硬编码 main — features/storage-crate.md #3
- export_memory atoms 不过滤 sensitive（口径与 list_atoms_all 相反，待拍板）— features/storage-crate.md #4
- assets 别名唯一性补 DB 约束；web fields String() 类型篡改 — features/assets-domain.md #1/#2
- project upsert_file 读-改-写非原子（对齐 update_doc 乐观锁模式）— features/project-domain.md #2
- project related 双向重复（反向查重）— features/project-domain.md #1

## T005 并发/竞态族

- append_session 缺 distill_status 守卫 — features/memory-domain.md #1
- circles create_entity 竞态 500（改 ON CONFLICT 幂等）— features/circles-domain.md #3
- deep_purge token 路径与到期 handler 双跑窗口 — features/jobs-logs-domain.md #1
- delete_provider is_default 竞态（并入删除 SQL）— features/settings-platform-domain.md #1
- credentials read_count 本地拼装 / 审计「谁」失真（MCP 记凭据名、HTTP 恒 console）— features/credentials-domain.md #1/#2
- tickets update 状态机无迁移校验（注释承诺未实现）— features/tickets-domain.md #4
- todos/tickets find_by_short_no 吞存储错误→DB 故障误报「不存在」（errors 宏观维确证为系统性断点）— features/todos-domain.md B2 + features/tickets-domain.md #2 + global/errors.md #5
- todos due 过滤 × cursor 翻页排序键不一致（首页 due_at ASC / 翻页切 updated_at DESC）丢行重行 — features/todos-domain.md B1
- study delete_item 越层直调 repo（不存在也 204、错误映 503，对照同文件 topic_delete 走 svc）— features/study-domain.md #2 + global/coupling.md #5
- distill atom_merge 不校验 keep_id 存在/active，victims 归档指向虚无不可逆 — features/distill-crate.md #2
- distill maintain_agent 工具错误 `?` 打死整个 job（对照 project_maintain 的 error 注入自纠模式）— features/distill-crate.md #3
- cg-bridge index()/sync() 对 client_upload 无守卫→上传产物被建空索引静默摧毁且来源翻 cloud_index — features/cg-bridge-crate.md #2
- change_credentials 改密+吊销其他会话两步独立写无事务，中间失败旧会话仍有效 — features/session-revocation.md #3 + features/auth-platform-domain.md #3

## T006 检索质量族

- wiki-engine search_opts 判空短路误杀向量通道（对照 core 修复补齐）— features/search-crate.md #2
- rerank 索引校验补查重（wiki-engine apply_llm_rerank + mcp search_all 两处）— features/search-crate.md #3 + features/llm-platform-domain.md #2
- search_all rerank 候选无排序取前 10 — features/llm-platform-domain.md #3
- QUERY_LOG_LOW_SCORE=0.017 阈值高于单通道 rank-1 满分 — features/search-crate.md #5
- wiki 检索 limit 负值守卫 — features/search-crate.md #4
- tickets list 负 limit 无守卫（对照 todos 同款补齐）— features/core-crate.md #1
- Circles graph 悬空引用（edges/relations 未过滤归档实体）— features/circles-domain.md #2

## T007 死代码/残余清理批（一批清）

- repo::insert_session、repo::count_running_extract、repo::credential::get_meta — features/storage-crate.md #5/#6
- llm::crypto::sha256_hex（孤儿导出；api 两份本地拷贝收编 core）— features/llm-crate.md #7
- cross_links::target_exists、GatewayLlm::budget_tokens、JobContext::llm_calls_made、assets::kind_label、mark_all_version_mismatch（疑似级逐个判）— 各 crate 篇
- CirclesEntityFullParams、api.setBase、withLib、parsing head 分支 + 重复断言、core/errors.rs Display 冗余条件、rhythm 无用 clone、transfer.rs 空注释占位
- api_keys.revoked_at 恒 NULL（删列或真软删除，待拍板）— features/auth-platform-domain.md #4
- dispatch 读写分类表幽灵条目（wiki.folders/purpose_set）+ circles help/hint 指向不存在的 delete — features/mcp-crate.md #4 + features/circles-domain.md #1
- codegraph register 工具文档仍称支持本地路径（AI 可见文案与实现矛盾）— features/codegraph-domain.md #2
- rrf_merge 疑似死（crate 公共导出零生产引用，判生死）— features/search-crate.md #8

## T008 P3 长尾与文档面（随做随记，不设期限）

- 各篇 P3 边界/一致性项（hex_decode、Bearer 大小写、UA 顶替空串、snippet Unicode、parse_html/docx 丢字、docx MIME 识别等）——见各篇详单
- aifriendly：baseline module-map 漂移（63 迁移→75、17 路由→19）+ CI 一致性断言 — global/aifriendly.md #5
- overdesign：wiki 多库残件处置、LLM Purpose 枚举收敛（**待拍板**，见 open-questions）
- jobs-crate heal 重摄取旧 chunk 残留（疑似）— features/jobs-crate.md #2
- CircuitBreaker HalfOpen 复位语义 — features/llm-crate.md #1
- errors/config 两宏观维补跑完成后通读排查（**2026-10-09 已完成，下两条为新增**）
- 错误注册表收编未完成：From<域错误>→EngramError 桥仅 WikiDocumentError 一批，Memory/Todo/Ticket 等域错误经 String 传出口丢结构化码（P006 Deferred 同源，并入该线跟进）— global/errors.md #9；LLM 用量记账失败仅 WARN 无监控信号 — global/errors.md #8；read_stats 连环吞错（.ok()?）静默降级 — global/errors.md #6（并入 T002 cg 线复核）
- config 面：散点 env 旋钮（≥8 个）绕过 Config 无总表、.env.example 死参数 AGENT_MEMORY_EMBEDDING_DIMENSIONS（P018 已登记待 docs-sync）+ 未收录散点变量、DSN 变量名三方不一、config.rs 过时 WARN 文案 — global/config.md #7-#11
- M1 审计遗留：`maintain_agent_merges_and_edits_persona_doc` 删了两条 job progress 回执断言（complete 守卫改造时同步删除，DB 效果断言全保留）——补记原因或恢复断言
- wiki-engine lint_deep 汇总明细被丢弃（report 只有计数，与模块文档承诺不符）— features/wiki-engine-crate.md #3
- promote.rs 第⑤步 mark_doc_promoted 失败留半态且幂等闸门挡死重试补写 — features/core-crate.md #2

> 逐条对账明细见 [reconciliation.md](reconciliation.md)（296 条三态标注，遗漏 14 / 误报 1）

## 里程碑

- M1：T001 五条 + 各自回归测试（本批最先）
- M2：T002/T003（安全+前端可用）
- M3：T004/T005（数据+并发）
- M4：T006/T007（检索+清理）
- 收官：docs-update 对齐 → docs-sync 同步 engram 镜像
