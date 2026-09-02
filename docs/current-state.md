# 当前状态（2026-09-01 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

origin/main 双 workflow 绿（编辑能力 + P-C 两阶段清空 + 登录态根修 + 圈子拆页全部落地）。
系统以**真数据**运行中：用户真实记忆（32+ 原子/场景/画像/实体），pi-xiamu 消费者 key 在役。

## 当日验证矩阵（活体）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt --check / clippy -D warnings | exit 0 / 0 errors |
| server | cargo test --workspace | 144 passed（36 套件，含圈子强化关系测试 + 关系回溯） |
| web | pnpm test / lint / build | 35/35 / 0 警告 / exit 0 |
| e2e | playwright journey（一次性栈 scripts/e2e-local.sh） | PASS 1 / FAIL 0（含快照差分自清） |
| 事实 | OpenAPI 活体 / 迁移 / 表 | **78 路径 / 96 方法注册**（GET 40/POST 42/PUT 4/PATCH 3/DELETE 7）/ **22 迁移** / **24 业务表** |

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

## 已知未了项

- e2e key 表历史积压（journey 现已自撤新 key；历史 revoked 行留存无害）
- skill 安装副本版本同步（~/.pi 侧非 git 跟踪，roadmap 0t）
