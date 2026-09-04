# 当前状态（2026-09-04 验证基线）

> 2026-08-30 初始化，09-01/09-03/09-04 多次全面更新。历史（CI 修复、Engram 重设计）见 git log 与根 docs/plantree/。

## 一句话状态

origin/main 产品更名 Engram 完成（仓库 trtyr/engram + crate engram-*）。cargo **181** 测试（37 套件）/ **29 迁移** / **88 路径 / 113 方法** / 27 业务表。
项目记忆第五域落地（三表 + 15 端点 + 唯一约束）+ 根 README 美化。**数据已由所有者主动清空，本地库已清理**。

## 当日验证矩阵

| 命令 | 结果 |
|---|---|
| cargo fmt --check | exit 0 |
| cargo clippy --workspace --all-targets -- -D warnings | 0 errors |
| cargo test --workspace | 181 passed / 0 failed（37 套件） |
| cargo run -q -p engram-api --bin openapi-dump | 88 路径 / 113 方法（GET 46/POST 47/PUT 7/PATCH 3/DELETE 10） |

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
