# P019 Roadmap

来源：docs/review/ 36 篇检查项（2026-10-09 审查），逐条发现以各篇 `features/<slug>.md` / `global/<dim>.md` 为详细依据（含 file:line 证据）。此处只登记任务级条目。

## T001 P1 正确性五条（先行）

| # | 内容 | 依据 |
|---|---|---|
| 1 | distill extract 重试重放→原子重复落库：persist_atoms 与 mark_sessions_done 同事务或按会话幂等 | features/distill-crate.md #1 |
| 2 | cg-bridge run_cli 超时不杀子进程（kill_on_drop）+ register clone 同构；复核 Timeout=Retryable 放大效应 | features/cg-bridge-crate.md #1 |
| 3 | cg_index/cg_sync 默认 300s visibility_timeout < 任务耗时→僵尸回收双执行；长任务设大超时或心跳续期；complete() 加 running 态守卫 | features/jobs-crate.md #1 |
| 4 | wiki-engine rebuild_all_links 无 page_type 过滤→系统页边瘫痪 orphan lint/repair | features/wiki-engine-crate.md #1 |
| 5 | tickets 全量导出被 core list `limit.min(500)` 静默截断（total 仍报全量） | features/tickets-domain.md #1 |

## T002 鉴权/越权族

- study:ro 全拒（require_study 对 ReadOnly 直接 Err）— features/auth-platform-domain.md #1
- /search 跨域越权：wiki-only key 可读 todos/tickets/entity（require_search 精确匹配绕开 domain_access）— features/scope-system-13.md #1 + features/llm-platform-domain.md #1
- original:ro 可执行 KV 写入 — features/mcp-crate.md #1
- llm/erase/cron 的 :ro 变体可签发但全域不可用 — features/scope-system-13.md #3
- migrate:ro 可 import（文档自认故意，待拍板口径）— features/scope-system-13.md #4
- tool_scope `_ => "memory"` 兜底→study/logs 工具发现面错位；删兜底改显式表 — features/mcp-crate.md #2 + global/coupling.md #2

## T003 前端可用性/正确性族

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

## T006 检索质量族

- wiki-engine search_opts 判空短路误杀向量通道（对照 core 修复补齐）— features/search-crate.md #2
- rerank 索引校验补查重（wiki-engine apply_llm_rerank + mcp search_all 两处）— features/search-crate.md #3 + features/llm-platform-domain.md #2
- search_all rerank 候选无排序取前 10 — features/llm-platform-domain.md #3
- QUERY_LOG_LOW_SCORE=0.017 阈值高于单通道 rank-1 满分 — features/search-crate.md #5
- wiki 检索 limit 负值守卫 — features/search-crate.md #4
- Circles graph 悬空引用（edges/relations 未过滤归档实体）— features/circles-domain.md #2

## T007 死代码/残余清理批（一批清）

- repo::insert_session、repo::count_running_extract、repo::credential::get_meta — features/storage-crate.md #5/#6
- llm::crypto::sha256_hex（孤儿导出；api 两份本地拷贝收编 core）— features/llm-crate.md #7
- cross_links::target_exists、GatewayLlm::budget_tokens、JobContext::llm_calls_made、assets::kind_label、mark_all_version_mismatch（疑似级逐个判）— 各 crate 篇
- CirclesEntityFullParams、api.setBase、withLib、parsing head 分支 + 重复断言、core/errors.rs Display 冗余条件、rhythm 无用 clone、transfer.rs 空注释占位
- api_keys.revoked_at 恒 NULL（删列或真软删除，待拍板）— features/auth-platform-domain.md #4
- dispatch 读写分类表幽灵条目（wiki.folders/purpose_set）+ circles help/hint 指向不存在的 delete — features/mcp-crate.md #4 + features/circles-domain.md #1

## T008 P3 长尾与文档面（随做随记，不设期限）

- 各篇 P3 边界/一致性项（hex_decode、Bearer 大小写、UA 顶替空串、snippet Unicode、parse_html/docx 丢字、docx MIME 识别等）——见各篇详单
- aifriendly：baseline module-map 漂移（63 迁移→75、17 路由→19）+ CI 一致性断言 — global/aifriendly.md #5
- overdesign：wiki 多库残件处置、LLM Purpose 枚举收敛（**待拍板**，见 open-questions）
- jobs-crate heal 重摄取旧 chunk 残留（疑似）— features/jobs-crate.md #2
- CircuitBreaker HalfOpen 复位语义 — features/llm-crate.md #1
- errors/config 两宏观维补跑完成后通读排查（进行中）

## 里程碑

- M1：T001 五条 + 各自回归测试（本批最先）
- M2：T002/T003（安全+前端可用）
- M3：T004/T005（数据+并发）
- M4：T006/T007（检索+清理）
- 收官：docs-update 对齐 → docs-sync 同步 engram 镜像
