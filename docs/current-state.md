# 当前状态（2026-09-04 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

origin/main 用户记忆 MCP 已落地（rmcp Streamable HTTP，/mcp 九工具）。cargo **189** 测试（38 套件）/ vitest 45。
**项目记忆第五域已实现**：三表（projects / project_locations / project_docs）+ 类型模板（开发四分类/调研六分类）+ 完整 API（88 路径）+ Web（列表 CRUD/多选/类型筛选 + 详情页 Wiki 式左树右内容树状图）。
:8090 开发栈在跑（am_dev 库，数据由所有者主动清空后重建）。

## 当日验证矩阵（活体）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt --check / clippy -D warnings | exit 0 / 0 errors |
| server | cargo test --workspace | 189 passed（38 套件） |
| web | pnpm run lint / tsc / test / build | 11 既有警告（WikiMarkdown） / 0 / 45 全过（9 文件）/ exit 0 |
| web | 入口 bundle | 285.60 kB（gzip 91.81），预算 350 内 |
| CI | gh run list（f8e1031） | CI + e2e FAIL（GitHub 支出限额，未启动） |
| 事实 | OpenAPI 活体 / 迁移 / 表 | **89 路径 / 115 方法注册**（GET 47/POST 47/PUT 8/PATCH 3/DELETE 10，另有 POST /mcp JSON-RPC 不进 OpenAPI）/ **30 迁移** / **27 业务表**（openapi-dump + am_dev 库实查） |

## 运行环境实况（2026-09-03 实查 + 所有者确认）

- **数据已由所有者主动清空**（非事故、无需恢复）：09-01 档案所述「用户真实记忆 + pi-xiamu key 在役」
  的运行态已终止。当前系统无生产数据在跑。
- 本地 PG 已清理：仅存 `postgres`（系统）与 `project_manage`（其他项目）；Engram 相关的
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
8. **双节律**（memory-rhythm）：AI 主动 + cron 兜底（外部 crontab 打 API，consolidate 日桶幂等）；心跳/status 端点 + 设置页节律 tab；cron scope 分权（status 可读 / heartbeat 2026-09-03 起为 **cron scope + via=cron 双条件**——scope 是软挡，via 是显式声明防线）
9. **测试隔离提级**（P11）：E2E_BASE 必填拒跑 + 一次性栈脚本 + journey 快照差分自清
10. **圈子强化 + 关系回溯**（0020/0021 迁移）：13 项强化——详情邻居/语义检索/社区聚类/实体历史/关系升级 A（entity_relations 类型化有向关系 + 蒸馏抽取）/全局时间轴/批量删除/实体导出；关系回溯：consolidate 对存量实体直接抽关系（无 session 重放兜底），常识关系 + 记忆明确关系
11. **权限收窄 + 二期三项**（2026-09-02）：AI 只写会话（收回直写 atom/entity/relation/attach），删实体/摘原子/删关系收进 erase scope；会话级敏感标记；文件批量导入（source=import + 蒸馏过滤对方观点）；过期自动降权/过滤（检索 ×0.5 + 注入硬过滤，归档先不做）；检索时间范围过滤（from/to，occurred_at 优先 NULL fallback created_at）
12. **Wiki+Knowledge 合并**（2026-09-02）：/knowledge 端点并入 /wiki 前缀（保留 /knowledge 兼容别名）；上传文档 ready 后自动织入 Wiki；前端融合成一个 Wiki 页（文档/页面/图谱/人审/提案/目标 tabs，删侧栏「知识库」项；DocumentsPane 组件被 Wiki 挂载）；图谱 Obsidian 化（hover 邻居高亮/拖拽/缩放控件/边按权重编码/位置缓存）
13. **Wiki Obsidian IA / 目录树重做**（2026-09-03，roadmap 0w）：0025 迁移 wiki_pages.**folder**（/ 分隔多级路径，蒸馏按 page_type 归文件夹，PUT /wiki/pages/{slug} 可改）；新增 **GET /wiki/proposals** 聚合端点（修前端 N+1 串行拉取）；前端 Wiki 页重做——目录树（折叠 localStorage 持久化 / 分割线拖拽 220-480px / role=tree 语义）+ 树/图双视图 + 收件箱/运维二级面板 + ?page= 深链自动展开所在 folder；新增 e2e wiki-ia.spec.ts；双链渲染三连修（递归 withWikilinks 深入行内 children / 取页 slug 宽容重查 / 404 显性提示）；阅读区排版 70ch→4xl 放宽
14. **配套修缮**（同期）：wiki lint 过时源 uuid cast + review resolve 未命中按 404；0022 迁移测试拆分（PgPool 跨连接 42P01 必挂修复）；Tabs 脏竖线改发丝网格
15. **级联删除审计凭证**（2026-09-03 收尾）：`WikiService::audit()` 接线 `delete_source_cascade`——破坏性操作落 jobs succeeded 行（kind=wiki_source_cascade_delete，payload 含 source_id + CascadeReport，best-effort 不阻断），与 memory 域「job 行即审计链」同哲学；cascade_test 补审计断言
16. **极端测试报告 25 项全修**（2026-09-03 晚，报告：/tmp/agent-memory-skill-test-report-2026-09-03.md，测试方独立 session 实测产出 3 P1/9 P2/13 P3）：① 安全收权——deep 全库清空仅限管理员会话（amk_ 一律 403，短语是公开常量防误不防蓄意）、heartbeat 加固 cron scope+via=cron 双条件、purge --agent 彻底清场（全部会话物理删除含 done/sensitive，先归档原子再删会话）；② 服务端修复——routing-suggest 解析鲁棒化（剥 fence/截 JSON 子串/诊断性错误）、会话轮次校验（speaker/非空/≤50000 字三入口同口径）、provider key ≥8、void 文案拆分 404/400、PUT/apply via 字段落 frontmatter、lint 大小写分级 case_mismatch + generation prompt v2；③ skill 侧批修（~/.pi，非 git）——rhythm-status/heartbeat CLI 入口、call/upload 去重+HTML 撞挡、防呆补齐、testing.md 全流程改会话路径、默认地址 8090、文档口径（去「双因子」、heartbeat 条件表述）；W-2 判定为审计链设计不改（文档写明语义）。验证：cargo 163（+5 新测试）、活体六项实测、CI+e2e 双绿（ce2d116）、独立 auditor 验收批准 + 测试方消费者复测 8 项全绿。
17. **项目记忆第五域落地**（2026-09-04，goal mtmmgwuu-d6g4rc）：0026 迁移建三表（projects / project_locations / project_docs）+ 类型模板（开发四分类「后端/前端/测试/规划」、调研六分类「待查/线索/资料/结论/疑点/证伪」，代码常量 + projects.categories 项目级可增删）+ 完整 API（15 endpoint、project scope 第八域、88 路径）+ Web（列表页 CRUD/多选批量删除/类型筛选 + 详情页 Wiki 式左树右内容：📍位置多主机 + 📄分类文档树 + 规划分类 + markdown 阅读编辑）。设计对齐见 docs/plantree/plans/project-memory/（0001-0005 五决策：第五域不单独建系统 / 类型驱动 / plan-tree 消融为规划分类 / 三表模型 / 记忆单一源 via skill）。验证：cargo 176（+6 集成测试）+ vitest 42（+5）+ 端到端（建项目→2 主机→2 分类文档→详情）+ 截图（project-list/detail-light.png）。

18. **位置元数据**（0027，2026-09-04）：project_locations 补 ip/os（多主机登记），概览页位置改为元数据卡片。
19. **项目域唯一约束 + 错误文案三问**（0028，2026-09-04）：projects.name UNIQUE + project_docs(project_id,category,title) UNIQUE，重复 409；401/404 补三问。
20. **产品更名 Engram**（2026-09-04，goal mtmzv6xt-b6u3ql）：GitHub 仓库 trtyr/engram、10 crate engram-*、品牌面全面 Engram 化、根 README 美化 + MIT LICENSE。

21. **knowledge 彻底并入 wiki**（2026-09-05，goal mtn6mye4-ql1zkm）：删 knowledge scope（八→七）、代码模块/类型归 wiki 命名（KnowledgeService→WikiDocumentService、knowledge_api→wiki_docs_api）、数据表改名（documents/chunks→wiki_documents/wiki_chunks，0029 迁移）、删 /knowledge/* 兼容别名、统一检索 knowledge 域标签并入 wiki；0030 迁移收尾——约束名归位（documents_pkey→wiki_documents_pkey 等 6 个）+ api_keys 默认 scopes 去 knowledge。

23. **MCP 管理面 + 密钥管理归位**（2026-09-05）：MCP 配置入 settings KV（key=`mcp`：enabled + disabled_tools，缺省全开无迁移）；`/mcp` 前置 gate 中间件——服务关闭对已认证客户端也 503（Bearer 之内、MCP 之前）；覆写 rmcp `list_tools`/`call_tool`——停用工具对 AI 隐身且调用被拒（管理端点仍展示全量）；`GET/PUT /settings/mcp`（PUT 校验未知工具名 400）；前端 MCP 页重排为管理台（状态条 + 域 Tabs 逐域工具开关列表，域归属后端同源 domain 字段），密钥管理收敛回设置页 Keys tab（签发表单补七 scope 选择器 + 表格 scopes 列）。验证：cargo 189（+2 toggle 用例）+ vitest 45（mcp.test 重写 3 用例）+ 真机冒烟（关服务 503 / 停用工具隐身+拒绝 / 恢复全开）。

24. **项目记忆 MCP 落地**（2026-09-05）：14 个 project_* 工具并入 `/mcp`（types/list/get/create/update/delete/batch_delete + location add/update/delete + doc add/get/update/delete），进程内直调 ProjectService，`project` scope 分权；tools/list 按 key scope 过滤（memory-only key 不见 project 工具，反之亦然——AI 看到的工具面与可调用集一致）；update 三件套（project/location/doc）MCP 层补丁式语义（不传不改，categories 替换式带提示）；doc add/update 前置分类校验（防笔误造出树上看不见的孤儿分类）；项目寻址支持 id 或 name（project_id_by_name）。顺修 core 两缺陷：改名撞名 409（原冒 500）、空/空白项目名 400（create/update 同口径）。验证：cargo 198（+9：project_mcp_test 八用例全旅程/改名冲突/批量级联/scope 分权/管理台开关 + project_test 改名回归）+ vitest 45 + 真机 E2E（curl JSON-RPC 建项目→登记→写文档→分类校验→补丁改状态→管理台域分组 9+14）。

25. **项目域审查问题修复**（2026-09-05，紧接 24 的审查结论）：① 归属校验——`/projects/{id}` 路径下 location/doc 的 get/update/delete 原先不核对资源归属（甲项目路径可寻址乙项目资源 id），现在 handler 层 owned_location/owned_doc 校验 project_id 一致，跨项目一律 404 且不泄露存在性；② 孤儿分类治理下沉 service——add_doc 校验 category ∈ project.categories（400 列出现有分类），update_doc 仅在换分类时校验（分类被移除后存量文档仍可原地编辑，不锁死），MCP 层预校验删除改依赖 service 单一真源。遗留：project_get 全量文档无分页，留待 context pack 协议一并设计。验证：cargo 200（+2：project_cross_project_access_blocked 六路 404 + 资源无损断言、project_doc_category_validation 四段矩阵）+ vitest 45。

## 已知未了项

- e2e key 表历史积压（journey 现已自撤新 key；历史 revoked 行留存无害）
- skill 安装副本版本同步（~/.pi 侧非 git 跟踪，roadmap 0t）
22. **用户记忆 MCP 落地**（2026-09-05）：官方 Rust SDK rmcp 3.2 Streamable HTTP 服务端宿主于 engram-server `/mcp`（无状态 + JSON 响应，复用 Bearer 中间件——每请求独立认证，key 吊销即刻生效）；九个 memory 域工具（context/search/list_atoms/list_sessions/get_session/write_session/append_session/forget/entities）进程内直调 MemoryService，instructions + 工具描述中文写明调用时机与编辑分权（AI 只写会话，纠错走蒸馏）；`GET /settings/mcp` 管理信息（与工具注册表同源）；前端新增 `/mcp` 页（端点信息 / Claude Code·Cursor·Claude Desktop 连接配置一键复制 / 工具清单 / MCP 密钥签发带 scope 选择）；远程部署 Host 白名单 `AGENT_MEMORY_MCP_ALLOWED_HOSTS`（默认 loopback 防 DNS rebinding）。验证：cargo +6（mcp_test 六用例：401 矩阵 / initialize+tools/list / 写读链 / scope 分权 / 管理端点）+ vitest +2。
