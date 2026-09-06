# 当前状态（2026-09-05 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

**五域 MCP + 分层收敛成型**：`/mcp` 一个端点承载 memory 九 + project 15 + skills 八 + wiki 八 + codegraph 五共 **45 工具**，按 key scope 分权（AI 看到的工具面与可调用集一致），管理台按域分组逐工具开点开看详情（描述/参数 Schema 与 tools/list 同源）。持久化全面收口 `engram-storage::repo`（core/api src 层零 sqlx），MCP 拆独立 crate（11 crates，与 HTTP 平级双适配器）。cargo **245** 测试 / vitest **55**（10 文件）/ 100 路径 / 132 方法 / 32 迁移 / 30 业务表。
:8090 开发栈在跑（am_dev 库，数据由所有者主动清空后重建）。

## 当日验证矩阵（活体）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt --check / clippy -D warnings | exit 0 / 0 errors |
| server | cargo test --workspace | 245 passed / 0 failed |
| web | pnpm run lint / tsc / test / build | 12 既有警告 / 0 / 55 全过（10 文件）/ exit 0 |
| web | 入口 bundle | 287.55 kB（gzip 92.46），预算 350 内 |
| CI | gh run list（f8e1031） | CI + e2e FAIL（GitHub 支出限额，未启动） |
| 事实 | OpenAPI 活体 / 迁移 / 表 | **104 路径 / 136 方法注册**（GET 57/POST 53/PUT 10/PATCH 3/DELETE 13，另有 POST /mcp JSON-RPC 不进 OpenAPI）/ **32 迁移** / **30 业务表**（openapi-dump + 库实查） |

## 运行环境实况（2026-09-03 实查 + 所有者确认）

- **数据已由所有者主动清空**（非事故、无需恢复）：09-01 档案所述「用户真实记忆 + pi-xiamu key 在役」
  的运行态已终止。当前系统无生产数据在跑。
- 本地 PG 已清理：仅存 `postgres`（系统）与 `project_manage`（其他项目）；Engram 相关的
  11 个库（agent_memory 老库 / am_design_audit 审计库 / 8 个一次性栈残留）已全部删除。
- :19180 design-audit 栈进程已停（审计报告已随仓库清理移除，见 git 历史 docs/design/）。需要本地栈时：
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

22. **用户记忆 MCP 落地**（2026-09-05）：官方 Rust SDK rmcp 3.2 Streamable HTTP 服务端宿主于 engram-server `/mcp`（无状态 + JSON 响应，复用 Bearer 中间件——每请求独立认证，key 吊销即刻生效）；九个 memory 域工具（context/search/list_atoms/list_sessions/get_session/write_session/append_session/forget/entities）进程内直调 MemoryService，instructions + 工具描述中文写明调用时机与编辑分权（AI 只写会话，纠错走蒸馏）；`GET /settings/mcp` 管理信息（与工具注册表同源）；前端新增 `/mcp` 页（端点信息 / Claude Code·Cursor·Claude Desktop 连接配置一键复制 / 工具清单 / MCP 密钥签发带 scope 选择）；远程部署 Host 白名单 `AGENT_MEMORY_MCP_ALLOWED_HOSTS`（默认 loopback 防 DNS rebinding）。验证：cargo +6（mcp_test 六用例：401 矩阵 / initialize+tools/list / 写读链 / scope 分权 / 管理端点）+ vitest +2。

23. **MCP 管理面 + 密钥管理归位**（2026-09-05）：MCP 配置入 settings KV（key=`mcp`：enabled + disabled_tools，缺省全开无迁移）；`/mcp` 前置 gate 中间件——服务关闭对已认证客户端也 503（Bearer 之内、MCP 之前）；覆写 rmcp `list_tools`/`call_tool`——停用工具对 AI 隐身且调用被拒（管理端点仍展示全量）；`GET/PUT /settings/mcp`（PUT 校验未知工具名 400）；前端 MCP 页重排为管理台（状态条 + 域 Tabs 逐域工具开关列表，域归属后端同源 domain 字段），密钥管理收敛回设置页 Keys tab（签发表单补七 scope 选择器 + 表格 scopes 列）。验证：cargo 189（+2 toggle 用例）+ vitest 45（mcp.test 重写 3 用例）+ 真机冒烟（关服务 503 / 停用工具隐身+拒绝 / 恢复全开）。

24. **记忆域 MCP v2 测试修复**（2026-09-05，fix/memory-mcp-v2 worktree）：黑盒测试报告（engram-mcp-test-report-v2.md）四项根因修复——① **检索零匹配短路**：FTS 零命中时向量腿收紧阈值（默认 0.45，`AGENT_MEMORY_VEC_FALLBACK_MAX_DISTANCE` 可调）+ 查询侧领域停用词（用户/什么等通用词不再让 FTS「处处命中」），不相关查询从满页噪声变空结果；② **distill=off 永久豁免**：off 会话落 metadata.distill=off，extract 认领/积压统计全部排除（此前会被任何蒸馏扫描顺带蒸掉）；③ **void 语义扩大**（P0-3 遗忘断层）：done 会话可 void，级联归档其蒸馏产物原子 + session_void_cascade 审计行；④ **context 预算**：budget_items 改各层独立上限（persona 不再挤占）、chars_used 按完整序列化计量（含 evidence_refs）；⑤ SearchHit.title 取内容前缀（不再恒 null）；⑥ 蒸馏 prompt v4/v2：隐私约定归 preference、场景措辞近似禁止另建 + 实体名包含关系近似归并 + 实体画像随 organize 链路生成。验证：cargo 195（+6 回归测试：off 豁免/void 级联/occurred_at 窗口/零匹配/title/预算）+ 8091 黑盒复验 11 项（不相关查询 0 结果、off 保持 pending、void 后检索立即失效、append 竞速 3/3、跨语言召回保留）。瞬态 401（N4）记录为启动窗口现象，warm 后 0/12 不可复现（新增两处观测均伴随服务重启窗口；怀疑与实例切换相关，未复现根因）。
   **embedding 模型切换 bge-m3 → Qwen3-Embedding-8B**（同日）：embed 链路全带 dimensions=1024（Qwen3 MRL 降维，列/索引不动）；实测几何压缩度与 bge 相当（不相关对 0.54-0.68）——压缩主因是语料同质（「用户+事实」短句）而非模型；新增查询侧指令包装 `AGENT_MEMORY_EMBED_QUERY_INSTRUCTION`（Qwen3 非对称检索必需，缺失导致跨语言召回归零）。am_dev 已切 Qwen 并全量重嵌（35 原子+11 场景），live 验证：问题式查询 PG 决策原子 17→**1**、跨语言 pet 命中、零匹配查询 0 结果。遗留两项已修（同日后续）：**逐措辞召回缺口**——兜底改「绝对天花板 + 相对间隔」双条件（`AGENT_MEMORY_VEC_FALLBACK_RELATIVE_MARGIN` 默认 0.15，最近邻一档内保留、远端尾巴裁掉），「宠物」查询经 search/context 均命中橘猫；**embed 静默失败**——try_embed 重试 3 次退避 + 失败显式记日志（查询降级纯 FTS / 文档缺向量不再无声）；连带发现并修：context 字符子预算——画像分面 ≤40%，evidence_refs 计入计量后不再挤光 atoms（context 宠物 atoms 0→1）。
25. **项目记忆 MCP 落地**（2026-09-05）：14 个 project_* 工具并入 `/mcp`（types/list/get/create/update/delete/batch_delete + location add/update/delete + doc add/get/update/delete），进程内直调 ProjectService，`project` scope 分权；tools/list 按 key scope 过滤（memory-only key 不见 project 工具，反之亦然——AI 看到的工具面与可调用集一致）；update 三件套（project/location/doc）MCP 层补丁式语义（不传不改，categories 替换式带提示）；doc add/update 前置分类校验（防笔误造出树上看不见的孤儿分类）；项目寻址支持 id 或 name（project_id_by_name）。顺修 core 两缺陷：改名撞名 409（原冒 500）、空/空白项目名 400（create/update 同口径）。验证：cargo 198（+9：project_mcp_test 八用例全旅程/改名冲突/批量级联/scope 分权/管理台开关 + project_test 改名回归）+ vitest 45 + 真机 E2E（curl JSON-RPC 建项目→登记→写文档→分类校验→补丁改状态→管理台域分组 9+14）。

26. **项目域审查问题修复**（2026-09-05，紧接 25 的审查结论）：① 归属校验——`/projects/{id}` 路径下 location/doc 的 get/update/delete 原先不核对资源归属（甲项目路径可寻址乙项目资源 id），现在 handler 层 owned_location/owned_doc 校验 project_id 一致，跨项目一律 404 且不泄露存在性；② 孤儿分类治理下沉 service——add_doc 校验 category ∈ project.categories（400 列出现有分类），update_doc 仅在换分类时校验（分类被移除后存量文档仍可原地编辑，不锁死），MCP 层预校验删除改依赖 service 单一真源。遗留：project_get 全量文档无分页，留待 context pack 协议一并设计。验证：cargo 200（+2：project_cross_project_access_blocked 六路 404 + 资源无损断言、project_doc_category_validation 四段矩阵）+ vitest 45。

27. **项目文档精确寻址读**（2026-09-05，所有者否决截断方案后重设计）：project_get 默认索引模式（docs 只给 id/分类/标题/content_chars，不带正文；include_content=true 无损全量；category 过滤）；新增 project_doc_search——grep 式跨文档按行检索（大小写不敏感子串，命中 doc_id/title/category/line/text，limit 上限）；project_doc_get 加 start_line/end_line 区间精读（1-based 含两端，输出恒带行号前缀便于连环寻址）与 with_line_numbers 全文行号——全文恒可得、零截断，工作流=索引看结构→搜索定位行号→区间精读。core 新增 read_doc_lines/search_doc_lines/DocLineHitDto。project 域工具 14→15。验证：cargo 201（+1：project_precise_addressing_read——索引/全量/过滤/搜索定位/区间端点含入/行号边界/空检索词）+ vitest 45。
28. **技能域第六域落地**（2026-09-05，feat/skills-mcp worktree）：0031 迁移建 skills + skill_revisions 两表（slug 唯一、tags GIN、版本快照保留 50 版）；SkillsService（frontmatter 容错解析支持 `>-`/`|` 块标量——现网 SKILL.md 真实形态、批量导入逐条报告 + 附带 tags、overwrite 覆盖、全量导出、q/tag/enabled 过滤、变更前自动快照 + 回滚前现状快照；scripts/import-skills.sh 目录一键灌入）；skills scope（七→八）+ 9 端点；MCP skills_* 六工具（list/get/create/update/delete/import，require_skills 分权，instructions 双域化）；Web 第十页 /skills（列表/新建/编辑/导入导出/版本回滚）+ 概览统计 + scope/域标签同步。验证：cargo 211（+skills 单测 12 + 集成 8 + MCP 3）/ vitest 52（+skills 7）/ 前后端门禁全绿。

29. **Wiki 域 MCP**（2026-09-05，feat/wiki-mcp）：单服务器扩为多域（memory + project + skills + wiki，工具名前缀即域，管理台按域分组自动出 Wiki tab）——`EngramMcpServer`（原 MemoryMcpServer 更名）新增八个 `wiki_*` 工具（mcp_wiki.rs 放参数结构/错误桥/实现辅助，`#[tool]` 方法落在 mcp.rs 同一 tool_router 块）：`wiki_search`（search_with_purpose 混合检索 + purpose）/ `wiki_list_pages`（瘦身去正文 content_omitted）/ `wiki_get_page`（slug 宽容匹配读全文）/ `wiki_write_page`（AI 通道 put_page，frontmatter.via="ai" 区分执行者；描述写明覆盖前先读原文）/ `wiki_ingest`（入队织入，返回 async=true 提示异步）/ `wiki_archive_query`（同标题幂等跳过）/ `wiki_graph` / `wiki_lint`；全部 wiki scope 分权（缺 scope 报 JSON-RPC 错误）；instructions 扩为多域（memory_* 管用户本人、wiki_* 管世界知识的域选择指引）；无参工具用空结构体 WikiNoParams（`Parameters<()>` schema 为 null 违反 MCP inputSchema 规范）。验证：cargo **193**（mcp_test 12 用例：wiki 工具清单/写读改搜列图 lint/问答存档幂等/织入/双域 scope 互拒/wiki 工具停用隐身+拒绝/管理台校验 wiki 工具名）+ vitest 46（mcp.test +1 Wiki 域 Tab 用例）+ 前端门禁四绿。

30. **三 feat 分支并入 main + 仓库收敛**（2026-09-05）：feat/project-memory-mcp、feat/skills-mcp、feat/wiki-mcp 三分支以 --no-ff 依次并入 main（b7ba722 / 67348b7 / 966d3f6）——四域工具链拼接、tool_scope 补 skills/wiki 分支、SERVER_INSTRUCTIONS 扩为四域、README/docs 多域口径统一；顺修 mcp_test toggle 断言（scope 过滤下 memory-only key 见 9 工具 9→8，skills 分支遗留的 15→14 与 project 分支的 scope 过滤矛盾）。三 worktree 撤除、四分支删除（含已合并的 fix/memory-mcp-v2，远端同步删除），仓库收敛为单分支 main。验证：cargo 234（40 套件）+ vitest 53 + fmt/clippy 全绿。

31. **持久化分层收敛**（2026-09-05）：SQL 全量收口 `engram-storage::repo`（8 域模块 145+ 仓储函数，行类型进 `engram-storage::models`；事务边界归 repo）——core/api src 层零 sqlx（测试夹具保留 dev-dep）；MCP 拆独立 crate **engram-mcp**（workspace 10→11 crates，与 HTTP 平级双适配器）；`Principal`/`AppState` 上移 core（auth/state）；api 加 `mcp_admin.rs`（/settings/mcp HTTP 壳）；jobs 加 `admin` 模块（deep-purge 两阶段读写/提案聚合收口）；ApiError.Database 换 `StoreError`。全量回归：cargo 245 / vitest 55 / fmt/clippy 零告警。

32. **技能 = 文件夹 + 三层消费**（2026-09-05）：0032 迁移建 `skill_files`（(skill_id, path) 唯一，写入侧校验禁 `..`/绝对路径/SKILL.md 本体）；skills_get 返回带 files 索引；MCP +2（skills_file_get/put，40→45 工具的部分）；HTTP 三层消费——①纯文本 MCP 读 ②单文件直下 `GET /skills/{slug}/file?path=…&raw=1`（Content-Disposition 落盘名）③整包 `GET /skills/{slug}/bundle`（zip：SKILL.md 自动还原 frontmatter + 全部文件）；导出含附属文件；消费形态指南织入工具描述（AI 自选通道）。云部署语义：文件是内容不是执行体——云端存发、客户端本地执行，服务端永不执行上传代码。

33. **CodeGraph 重做 + MCP 工具面动态化**（2026-09-05）：CodeGraph 接入任务队列（cg_index/cg_sync job，POST 返回 202——废除同步阻塞 10 分钟）；新增 DELETE（git clone 工作目录一并清理）与 GET /codegraph/status（CLI 可用性，Windows spawn 适配 .cmd shim）；**MCP +5 codegraph_* 工具（38→45，五域）**，codegraph_list 描述织入动态项目清单；新增 `GET /codegraph/projects/{id}/graph` 双模式——无 symbol = 文件级全图（rusqlite 只读 CLI 索引库聚合跨文件依赖），带 symbol = callers/callees 子图归一 nodes+edges；前端代码图谱页重做（CLI 状态条 / indexing 自动轮询 / 删除 / sigma 调用图弹窗）；修 stats 字段错位（CLI fileCount/nodeCount/edgeCount → 前端 files/symbols/edges）与同源重复注册。CLI 安装 `npm i -g @colbymchenry/codegraph@1.5.0`。MCP 工具描述动态化（skills_list/project_list/codegraph_list 清单织入，管理台与 tools/list 同源）。验证：cargo **245** / vitest **55** / 前后端门禁全绿 + 真实 CLI 端到端（注册 engram→异步索引→查询→调用图→MCP tools/list 45）。

## 已知未了项

- e2e key 表历史积压（journey 现已自撤新 key；历史 revoked 行留存无害）
- skill 安装副本版本同步（~/.pi 侧非 git 跟踪，roadmap 0t）
