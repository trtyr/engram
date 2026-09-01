# 当前状态（2026-09-01 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

origin/main 双 workflow 绿（编辑能力 + P-C 两阶段清空 + 登录态根修 + 圈子拆页全部落地）。
系统以**真数据**运行中：用户真实记忆（32+ 原子/场景/画像/实体），pi-xiamu 消费者 key 在役。

## 当日验证矩阵（活体）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt --check / clippy -D warnings | exit 0 / 0 errors |
| server | cargo test --workspace | 138 passed（36 套件） |
| web | pnpm test / lint / build | 32/32 / 0 警告 / exit 0 |
| e2e | playwright journey（本地栈 :19180） | PASS 1 / FAIL 0（含收尾自清） |
| 事实 | OpenAPI 活体 / 迁移 / 表 | **67 路径**（GET 31/POST 29/PATCH 1/DELETE 6）/ **19 迁移** / **22 业务表** |

## 2026-08-30 基线以来的大事记

1. **实体层**（0015 迁移）：entities + atom_entities，蒸馏自动抽取，圈子页浏览，实体进检索与 context
2. **时间表达力**（0016）：atoms.occurred_at/valid_until，实体 kind +place，LLM 抽相对时间→绝对日期
3. **敏感与清空**（0017）：atoms.sensitive 全链排除；void/purge/export；F3 快照收敛；F4 敏感清退
4. **编辑能力**（0018）：AI/用户分权、atom_revisions 留痕、manually_edited 钉住（清退>钉住>自动重写）
5. **P-C 两阶段清空**（0019）：arm 5 分钟冷却 → token 执行 / cancel 后悔药；三次清空事故的架构级防线
6. **圈子拆独立页**：/circle 与代码图谱对称；用户记忆页回归纯梯子（五 tab 默认会话）
7. **登录态根修**：JobStatus 补 cancelled 变体（503 连环误判），探活改 401-only

## 已知未了项

- 自动节律钩子（pi extension）——roadmap 缓做项，方向已定（用户 wishlist #1）
- e2e key 表历史积压（journey 现已自撤新 key；历史 revoked 行留存无害）
- skill 安装副本版本同步（~/.pi 侧非 git 跟踪，roadmap 0t）
