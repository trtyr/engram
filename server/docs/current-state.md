# 当前状态（2026-09-01 验证基线）

> 2026-08-30 初始化后的首次全面更新。历史（CI 修复、Engram 重设计）见 git log 与根 docs/plantree/。

## 一句话状态

工作树干净，origin/main 双 workflow 绿。cargo **141** 测试 / vitest **35** / 19 迁移 / 69 路径 / 22 业务表。
生产栈 :19180 跑真数据（用户真实记忆 + pi-xiamu 消费者 key 在役）。

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

## 运行中的真数据

- 记忆域：用户真实记忆运行中（原子/场景/画像/实体），2 条敏感带标
- 消费者：pi-xiamu key（memory+llm+erase）在役；v2 只读 key 留用
- LLM：newapi 网关（MiniMax-M3 chat + bge-m3 embed）

## 已知未了项

- 自动节律钩子（pi extension 开场注入/收尾写回）缓做——roadmap 0t（cron 兜底已落地，钩子管「会话内实时」层）
- skill 安装副本（~/.pi）非 git 跟踪，版本同步是已知风险——roadmap 0t
- 无 provider 的 CI e2e 走部分旅程（蒸馏断言跳过，单测覆盖）
