# 当前状态（2026-09-03 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

origin/main a3b6a2d 双 workflow 绿（CI + e2e）；本地全量门禁当日全绿（cargo 158 / vitest 37 / build 0）。
Wiki 域 09-03 完成 Obsidian IA 重做（目录树 + 树/图双视图 + 双链三连修，roadmap 0w 28 项审计全修）；
级联删除补审计凭证（wiki_source_cascade_delete 落 jobs 行）。**数据已由所有者主动清空**（见运行环境实况）。

## 当日验证矩阵（活体）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt --check / clippy -D warnings | exit 0 / 0 errors |
| server | cargo test --workspace | 158 passed（36 套件） |
| web | pnpm run lint / tsc / test / build | 0 警告 / 0 / 37 全过（6 文件）/ exit 0 |
| web | 入口 bundle | 284.43 kB（gzip 91.43），预算 350 内 |
| CI | gh run list（a3b6a2d） | CI + e2e 双 success（GitHub 实查） |
| 事实 | OpenAPI 活体 / 迁移 / 表 | **80 路径 / 98 方法注册**（GET 41/POST 43/PUT 4/PATCH 3/DELETE 7）/ **25 迁移** / **24 业务表**（openapi-dump + am_design_audit 库实查） |

## 运行环境实况（2026-09-03 实查 + 所有者确认）

- **数据已由所有者主动清空**（非事故、无需恢复）：09-01 档案所述「用户真实记忆 + pi-xiamu key 在役」
  的运行态已终止。当前系统无生产数据在跑。
- 本地 PG 已清理：仅存 `postgres`（系统）与 `project_manage`（其他项目）；agent-memory 相关的
  11 个库（agent_memory 老库 / am_design_audit 审计库 / 8 个一次性栈残留）已全部删除。
- :19180 design-audit 栈进程已停（审计证据落档 docs/design/）。需要本地栈时：
  `CREATE DATABASE` + `cargo run`（迁移自动跑，见 run-and-deploy.md）。

## 2026-08-30 基线以来的大事记

1. **实体层**（0015 迁移）：entities + atom_entities，蒸馏自动抽取，圈子页浏览，实体进检索与 context
2. **时间表达力**（0016）：atoms.occurred_at/valid_until，实体 kind +place，LLM 抽相对时间→绝对日期
3. **敏感与清空**（0017）：atoms.sensitive 全链排除；void/purge/export；F3 快照收敛；F4 敏感清退
4. **编辑能力**（0018）：AI/用户分权、atom_revisions 留痕、manually_edited 钉住（清退>钉住>自动重写）
5. **P-C 两阶段清空**（0019）：arm 5 分钟冷却 → token 执行 / cancel 后悔药；三次清空事故的架构级防线
6. **圈子拆独立页**：/circle 与代码图谱对称；用户记忆页回归纯梯子（五 tab 默认会话）
7. **登录态根修**：JobStatus 补 cancelled 变体（503 连环误判），探活改 401-only
8. **双节律**（memory-rhythm）：AI 主动 + cron 兜底（外部 crontab 打 API，consolidate 日桶幂等）；心跳/status 端点 + 设置页节律 tab；cron scope 分权（status 可读 / heartbeat+via:cron 专属，杜绝 AI 伪造）
9. **测试隔离提级**（P11）：E2E_BASE 必填拒跑 + 一次性栈脚本 + journey 快照差分自清
10. **圈子强化 + 关系回溯**（0020/0021 迁移）：13 项强化——详情邻居/语义检索/社区聚类/实体历史/关系升级 A（entity_relations 类型化有向关系 + 蒸馏抽取）/全局时间轴/批量删除/实体导出；关系回溯：consolidate 对存量实体直接抽关系（无 session 重放兜底），常识关系 + 记忆明确关系
11. **权限收窄 + 二期三项**（2026-09-02）：AI 只写会话（收回直写 atom/entity/relation/attach），删实体/摘原子/删关系收进 erase scope；会话级敏感标记；文件批量导入（source=import + 蒸馏过滤对方观点）；过期自动降权/过滤（检索 ×0.5 + 注入硬过滤，归档先不做）；检索时间范围过滤（from/to，occurred_at 优先 NULL fallback created_at）
12. **Wiki+Knowledge 合并**（2026-09-02）：/knowledge 端点并入 /wiki 前缀（保留 /knowledge 兼容别名）；上传文档 ready 后自动织入 Wiki；前端融合成一个 Wiki 页（文档/页面/图谱/人审/提案/目标 tabs，删侧栏「知识库」项；DocumentsPane 仍住 Knowledge.tsx）；图谱 Obsidian 化（hover 邻居高亮/拖拽/缩放控件/边按权重编码/位置缓存）
13. **Wiki Obsidian IA / 目录树重做**（2026-09-03，roadmap 0w）：0025 迁移 wiki_pages.**folder**（/ 分隔多级路径，蒸馏按 page_type 归文件夹，PUT /wiki/pages/{slug} 可改）；新增 **GET /wiki/proposals** 聚合端点（修前端 N+1 串行拉取）；前端 Wiki 页重做——目录树（折叠 localStorage 持久化 / 分割线拖拽 220-480px / role=tree 语义）+ 树/图双视图 + 收件箱/运维二级面板 + ?page= 深链自动展开所在 folder；新增 e2e wiki-ia.spec.ts；双链渲染三连修（递归 withWikilinks 深入行内 children / 取页 slug 宽容重查 / 404 显性提示）；阅读区排版 70ch→4xl 放宽
14. **配套修缮**（同期）：wiki lint 过时源 uuid cast + review resolve 未命中按 404；0022 迁移测试拆分（PgPool 跨连接 42P01 必挂修复）；Tabs 脏竖线改发丝网格
15. **级联删除审计凭证**（2026-09-03 收尾）：`WikiService::audit()` 接线 `delete_source_cascade`——破坏性操作落 jobs succeeded 行（kind=wiki_source_cascade_delete，payload 含 source_id + CascadeReport，best-effort 不阻断），与 memory 域「job 行即审计链」同哲学；cascade_test 补审计断言

## 已知未了项

- e2e key 表历史积压（journey 现已自撤新 key；历史 revoked 行留存无害）
- skill 安装副本版本同步（~/.pi 侧非 git 跟踪，roadmap 0t）
