# P019 待拍板

（只留未决；拍板后移入 decisions/ 并更新 roadmap）

## Q001 · React Query：引入还是移除？

`@tanstack/react-query` 在依赖中但全仓零使用，全站手写 `load()` 全量重拉。
二选一：真正引入改造数据层，或删依赖维持现状。（global/frontend.md #1）

## Q002 · wiki 多库残件处置

产品已官宣单库终局，但多库基础设施仍在付费（每请求 resolve、每库 purpose、patrol 只巡 main 的 bug 即源于此）。
选项：①冻结（library_id 恒 main，删 handler resolve 调用）②真做回多库 API。（global/overdesign.md #1-4；T005 内 patrol 硬编码修复与此相关，同源还有 wiki-engine 跨库链接写入库内边表 — features/wiki-engine-crate.md #4）

## Q003 · LLM Purpose 枚举收敛

11 档 vs 路由只用 Embed/chat 两分。砍到 3 档 / 补真实路由表 / 维持（记账标签够用）。（global/overdesign.md #5）

## Q004 · migrate:ro 语义

migrate:ro 可执行 import（全系统写入）。注释自认故意（同步钥匙），但与 :ro 全局语义相悖。放行维持 or 拒绝 :ro 变体签发？（features/scope-system-13.md #4）

## Q005 · export_memory 含 sensitive atoms？

迁移包导出不过滤 sensitive，与业务面导出口径相反。确认迁移场景是否豁免敏感面。（features/storage-crate.md #4）

## Q006 · api_keys.revoked_at

恒 NULL 残留字段：物理删列 or 改真软删除（顺带补吊销审计）？（features/auth-platform-domain.md #4）
