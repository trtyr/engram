# 当前状态（2026-09-08 验证基线）

> 初始化 2026-08-30；09-01/03/04/06 多轮更新；**2026-09-08 大版本刷新**（多库 Wiki、待办批量、
> 会话恢复、MCP 七工具、字体定版）。历史演进见 git log 与根 [docs/plantree](../../docs/plantree/)。

## 一句话状态

Rust 侧 **11 crates** 七域服务（memory / wiki（**多库**）/ codegraph / project / skills / todos）+
**MCP 适配器**（独立 crate engram-mcp；渐进式发现：**7 个入口工具 = 六域 + 跨域 search_all，
共 63 个域内 action**——L0 描述内嵌目录 + L1 `action="help"` 手册 + L2 错误自愈；scope 分权 +
整域/单操作两级开关；写操作不回显正文只回元数据）+ 持久化收口 `engram-storage::repo`
（core/api/mcp src 层零 sqlx，HTTP 与 MCP 双适配器共用 core 服务）+ embedding = Qwen3-Embedding-8B
（查询侧指令包装 + 兜底双条件）。cargo **243** 测试 / **37 迁移** / **34 业务表**。前端 vitest **59**。

## 各域当前形态

### memory 用户记忆
- 四层蒸馏：L0 raw_sessions → L1 atoms（八类）→ L2 scenarios → L3 persona_aspects（七分面）；
  实体（entities + atom_entities + entity_relations）横向串联。
- 写入校验全集：turns 逐条校验（speaker/text 上限/ts 可解析）、distill 枚举、负 limit、NUL 拒绝；
  凭据类内容蒸馏**主动不落原子**（extract 提示词 v5 成文策略）；宠物等个体名 → person 实体。
- 遗忘三态：void（作废，active+superseded 原子级联归档，pre_void 状态存 metadata）/
  erase（物理删除 + 原子级联归档 + 孤儿实体清扫 + 场景收敛）/ **restore（撤销作废，原子按
  superseded_by 各归其位）**。批量擦除/批量恢复（单批 ≤200）。
- **画像**：F4 清退写空版本作退休标记；`persona_current` 排除退休空版本（Dashboard 计数与
  画像页同源）；search 默认剥 L3 evidence_refs（include_evidence 开关）。
- `list_atoms` 默认 active（"all" 显式全量）；分页 keyset；score 3 位舍入。
- 一句话记忆 `remember`（单轮 write_session + auto 蒸馏）。

### wiki 知识库（2026-09-08 起多库）
- **库 = 一级命名空间**：`wiki_libraries` 表（main 主库自动创建，不可删）；pages/sources/
  documents/chunks/links/review_items/page_versions/insight_dismissals 全挂 `library_id`；
  slug/sha 唯一降为库内复合唯一；**purpose 每库一份**（settings key `wiki_purpose:{lib}`）。
- 语义：同 slug 跨库共存合法；织入（ingest）产物落 source 所在库；检索/图谱/lint/原料/版本
  全部库内收窄；级联删除（摘源/删页）按库；跨库搜索用统一检索 /search（全库并集）。
- 版本化：覆盖/删除自动快照（每 slug 留 50 版）；versions / version_content / restore_version；
  删除页可从快照重建（版本号接续快照史）。
- 通道：write_page（human/ai，frontmatter.via）/ ingest 两步流水线（analyze→generate，sha 库内
  去重、三态结果、0 产物提示）/ archive_query 幂等存档 / 文档上传（URL+文件，解析分块嵌入，
  可再织入）。
- 治理：lint（死链/孤页/缺源/相似目录）、图洞察（意外连接/孤立页/稀疏社区/桥节点 + dismiss）、
  4 信号相关性权重、Louvain 社区、purpose 人审建议。
- **多库入口**：HTTP 全部 `/wiki/*` 接 `?lib=<slug>`；MCP wiki 全 action 可选 `library` 参数 +
  `libraries` 列表操作；库管理端点 GET/POST `/wiki/libraries`、DELETE `/wiki/libraries/{slug}`
  （main 不可删、非空需 force）。

### projects / skills / todos / codegraph
- projects：目标/多主机位置登记/分类文档树；行级补丁 doc_patch（replace/insert/delete）；
  doc_get 区间精读恒带行号。
- skills：SKILL.md + 附属文件（路径校验禁盘符冒号）；三种消费形态（MCP 读/raw 单文件/zip 整包）；
  版本快照（每语义变更自动留，50 版）+ MCP versions/restore；非 ASCII 名必须显式 slug。
- todos：open/done/archived + 优先级/标签/截止；批量擦除/批量恢复（单批 ≤200）；done 幂等；
  keyset 分页（三元组游标）；导出。
- codegraph：CLI 1.5.0 pin 桥；search/explore（**默认符号大纲，include_source=true 才带源码**）/
  node/callers/callees/impact；git URL 与本地路径注册（路径按服务端 FS 校验）；纯 README 仓库
  0 符号为正常行为（index 提示已说明）。

### MCP 工具面（详见 [mcp.md](mcp.md)）
- **7 入口工具 / 63 域内 action**：memory 10、projects 16、skills 10、wiki 15、todos 6、
  codegraph 6 + 跨域 search_all。
- 写操作统一不回显正文（content_omitted + content_chars）；三级渐进式发现；
  工具/操作两级停用开关；tools/list 按 key scope 过滤 + 动态资产清单（技能/项目/代码库）
  织入描述；SERVER_INSTRUCTIONS 含各域用法与分权规则。

### Web 门户
- 记忆（会话批量勾选恢复/擦除、单会话恢复按钮、原子/画像/场景/检索）、圈子（实体星系）、
  Wiki（**库切换器 + 库管理** + 目录树 + 图谱 + 收件箱 + 洞察 + 人审 + 提案 + 目标）、
  代码图谱、项目、技能、待办、任务页、MCP 管理、设置。
- 全局：Cmd+K 面板、统一跨域 /search、深链、暗/亮主题；
  **字体**：英文 JetBrains Mono（本机 Nerd 变体优先）+ 中文霞鹭文楷（切片 woff2 按需加载），
  等宽场景中文落文楷 Mono；确认弹窗全站应用内化（components/confirm.tsx，禁原生 confirm）。

## 当日验证矩阵

| 命令 | 结果 |
|---|---|
| `cargo test --workspace`（server/） | **243 passed / 0 failed**（另 wiki-engine 内 33 含于其中） |
| `cargo clippy --workspace --all-targets` | 0 警告 |
| `cd web && pnpm tsc -b && pnpm vitest run && pnpm oxlint src && pnpm build` | 0 错误 / 59 passed / 成功 |
| 双库实机验收（HTTP 全链） | 12/12：建库、同名 slug 隔离、检索收窄、purpose 独立、未知库 404、织入落库、非空拒删、force 级联、main 不受波及 |
| 多库黑盒专项（scripts/zztest_m10.py） | 27/27 PASS（报告：workspace/engram-mcp-test-report-m10.md） |

## 已知边界（非缺陷）

- codegraph 宏内调用不可见（上游 CLI 局限）；纯 README 仓库索引 0 符号（CLI 正常行为）。
- 凭据类内容蒸馏 0 产物（有意策略，提示词 v5 成文）。
- 敏感会话蒸馏可能 0 产物（LLM 侧保守，继承 sensitive 标记）。
- MCP 无 provider 配置工具（有意设计，报错引导 Web 设置页）。
- main 库不可删除（缺省调用落点保护）；非空库删除需 force。
- PG 全库 fsync 恢复在异常停机后可达 10+ 分钟（测试残留库放大）——测试后建议清理 am_test_% 库。
