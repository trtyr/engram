# 当前状态（2026-09-05 验证基线）

> 2026-08-30 初始化，09-01/09-03/09-04 多次全面更新。历史（CI 修复、Engram 重设计）见 git log 与根 docs/plantree/。

## 一句话状态

仓库收敛为单分支。Rust 侧全貌：**11 crates** 五域服务（memory / wiki / codegraph / project / skills）+ **五域 MCP 适配器**（独立 crate engram-mcp；`/mcp` 承载 memory 九 + project 15 + skills 八 + wiki 八 + codegraph 五共 45 工具，scope 分权 + 管理台开关，tools/list 按 key scope 过滤 + 动态资产清单织入描述）+ 持久化收口 `engram-storage::repo`（core/api src 层零 sqlx，双适配器共用 core 服务）+ embedding 切 Qwen3-Embedding-8B（查询侧指令包装 + 兜底双条件）。cargo **250** 测试 / **34 迁移** / **31 业务表**（+admin_account 单行管理员账号、entities.archived_at 实体归档标记）。
**MCP 黑盒测试战役收尾**（zcode 五轮黑盒 + 六轮回归，90+ 真实调用）：16 项发现全部收口——D1 蒸馏静默失败（base_url /v1 双重约定 404 根因，normalize_base_url 规范化 + 未配 LLM manual 显式报错带 Web UI 引导）、D2 并发丢字段（COALESCE 部分更新）、D3 技能路径冒号绕过、D4/D5/D8 wiki 链接索引/幂等/title 同步（含 /wiki/links/rebuild 存量回填）、D6 sessions 元数据列表、D7 YAML 块列表 tags、D9/D10 删除工具、D11 织入去重三态、D13 空 target、D15 遗忘级联到 L2/L3、D16 遗忘级联到实体层（archived_at + 复活）；D12 上游 CLI 局限、D14 按用户决定不补 MCP 配置工具（D1 报错已带 Web UI 引导，闭案）。蒸馏全链路实测：manual 写会话 5s 出原子，质量佳。MCP 黑盒测试报告（zcode，90+ 调用）14 缺陷修复：D1 蒸馏未配 LLM 显式报错、D2 doc_update COALESCE 部分更新、D3 技能文件路径冒号拒绝、D4/D8 wiki 写页重算 links+frontmatter.title 同步、D5 archive 双保险幂等、D6 sessions 列表摘要、D7 frontmatter 块列表 tags、D9/D10 codegraph/wiki 删除工具、D13 空 target 拒绝、D14 MCP +2 llm 工具（45→49）。

## 当日验证矩阵

| 命令 | 结果 |
|---|---|
| cargo fmt --check | exit 0 |
| cargo clippy --workspace --all-targets -- -D warnings | 0 errors |
| cargo test --workspace | 245 passed / 0 failed |
| cargo run -q -p engram-api --bin openapi-dump | 104 路径 / 136 方法 |

## 2026-08-30 基线以来的落地（按主题）

| 主题 | 内容 | 代表提交 |
|---|---|---|
| 实体层 | 0015 迁移 + 9 API + 蒸馏抽取 + 圈子页 | 1067a21/8bfd330 |
| 记忆模型九修 | 抽取放宽/实体档案/检索四层/re-embed/人审队列/一致性 | 595d5be..3dd210a |
| 消费者契约面 | context_pack 实体透镜 + amk_ 全旅程测试 | 47c2caf/9b0e176 |
| 时间表达力 | occurred_at/valid_until/place kind/今天锚 | c75f668 |
| 敏感与清空 | sensitive 全链 + void/purge/export + F3 快照收敛 + F4 清退 | 3e205df/bfa2e3f/b7203e6 |
| 编辑能力 | 分权/留痕/钉住 + Web 编辑面 | 0189b1b/534d701 |
| 清空防线 | P-A 互斥/P-B 直写聚类/P-C 两阶段 | b044bf5/1472832 |
| 登录态根修 | JobStatus cancelled + 探活 401-only | b5ac042 |
| 圈子拆页 | /circle 独立页 + Memory 回归纯梯子 | 36342e8 |
| e2e 自清 | journey 收尾清 agent/实体/key | a7ae4f4 |
| 测试隔离 | E2E_BASE 必填 + 一次性栈 + 差分自清 | 399deab |
| 双节律 | cron 兜底 + 心跳/status + cron scope 分权 | 775aea2/c1f877f/d7a8345 |
| 会话敏感 + 直写残留 | raw_sessions.sensitive 蒸馏继承 + 无溯源原子打标 origin（0023/0024 迁移） | f6fca87/73d0d65 |
| Wiki+Knowledge 合并 | 端点并入 /wiki（兼容别名）+ 上传自动织入 + 前端融合一个 Wiki 页 + 图谱 Obsidian 化 | bb98d07/c37ede3/a69fcb3/c1b5804 |
| 供应商单模型 | llm_providers models→model_id+capability（0022）+ AI 路由建议 + 批量删除 | 50dc2d4 |
| 权限收窄 | AI 直写加工权收回（atom/entity/relation/attach 403）+ erase 分权 + atom 输入校验 | 176b070/fa88777/e48cdc5 |
| 二期三项 | 过期降权/过滤 + 文件批量导入（source=import）+ 检索时间范围过滤 | d9316b1/1f082e7/9c45889 |
| **Wiki 目录树** | **0025 wiki_pages.folder**（/ 分隔层级，蒸馏按 page_type 归文件夹，PUT 可改）+ **GET /wiki/proposals** 聚合端点（修 N+1）+ wiki lint uuid cast/review 404 + 0022 测试拆分（PgPool 42P01） | e568732/adbc57e/952035f/a1aea80 |
| 双链健壮性 | 取页 slug 宽容重查（标题原文双链不再 404） | 6d1fadc |
| 项目记忆第五域 | 0026 三表（projects/locations/docs）+ 类型模板 + 15 端点 + Web 列表/详情 | 2661272/cdc27e1 |
| 位置元数据 | 0027 project_locations.ip/os（多主机登记） | 270faab |
| 项目域唯一约束 | 0028 name/doc title UNIQUE + Conflict 409 + 错误文案三问 | dffdd9a |
| 产品更名 Engram | 仓库 trtyr/engram + 10 crate engram-* + 品牌面 + 根 README | 23cbdbb |
| **持久化分层收敛** | SQL 全量收口 engram-storage::repo（8 域仓储 145+ 函数；core/api src 零 sqlx）；engram-mcp 独立 crate（11 crates，与 HTTP 平级双适配器）；Principal/AppState 上移 core；jobs 加 admin 管理面模块 | 本轮（无 commit 仓库） |
| **技能 = 文件夹 + 三层消费** | 0032 skill_files（(skill_id,path) 唯一 + 路径校验）；skills_get 带 files 索引；MCP +2 skills_file_get/put；HTTP raw 单文件直下 + bundle 整包 zip（SKILL.md frontmatter 还原）；工具描述织入消费形态指南 | 本轮 |
| **CodeGraph 重做** | index/sync 接任务队列（202 入队废除同步 10min）；DELETE/status/graph 三端点；graph 双模式（无 symbol=文件级全图 rusqlite 只读聚合 / 带 symbol=callers+callees 子图归一）；MCP +5 codegraph_*（45 工具五域）；Windows .cmd spawn 修复；stats 字段归一修复；同源查重 | 本轮 |

## 当日落地：级联删除审计凭证

- `WikiService::audit(kind, payload)`（jobs 表 succeeded 行，best-effort）接线到
  `delete_source_cascade`——kind=`wiki_source_cascade_delete`，payload 含 source_id + CascadeReport；
  破坏性操作不再无痕（与 memory 域「job 行即审计链」同哲学）。cascade_test 补落行断言。

## 运行环境实况

- **数据已由所有者主动清空**（2026-09-03 确认，非事故）：无生产数据在跑。
- 本地 PG 已清理：仅存 postgres / project_manage；Engram 相关 11 库（含 agent_memory 老库、
  am_design_audit 审计库）已删。:19180 审计栈进程已停。重建本地栈：
  `psql -c "CREATE DATABASE am_dev"` + `AGENT_MEMORY_DATABASE_URL=… cargo run`（迁移自动跑）。

## 已知未了项

- skill 安装副本（~/.pi）非 git 跟踪，版本同步是已知风险——roadmap 0t
- 无 provider 的 CI e2e 走部分旅程（蒸馏断言跳过，单测覆盖）
