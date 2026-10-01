# 风险 → 处置映射（22 项工作视图)

> 详情与行号看档案风险篇 01a0e6f6-04d2-7b03-b128-29863af0e008。本表只追「去哪了」。

## P0 会咬人

| # | 风险 | 处置 |
|---|---|---|
| 1 | backup.sh 兜底空卷 | **T001（Next）** |
| 2 | 弱口令静默回落 | Deferred（改拒启行为需确认不影响本地 dev 习惯） |
| 3 | 生产反代无 IaC | 挂起（需登录生产机实查，非本仓代码能解） |

## P1 语义债

| # | 风险 | 处置 |
|---|---|---|
| 4 | 敏感语义分裂 | **决策 001 已定案**（边界三项拍板），T004 In Progress、T005 回填待确认实例 |
| 5 | prompt_version 仅 L3 落库 | Deferred（补列涉迁移回填） |
| 6 | HTTP/MCP 读口径不对称 | Deferred（先定基准方向） |

## P2 规模

| # | 风险 | 处置 |
|---|---|---|
| 7 | 10k 页 pages_list/graph 慢 | Deferred（独立 plan，benchmark 先行） |
| 8 | 前端全量拉取无缓存 | Deferred（随性能 plan） |
| 9 | extract claim 无 LIMIT | 随 T004 敏感批次顺手评估 |
| 10 | MCP list_tools 每请求双读+重建 router | Deferred（微优化） |

## P3 便宜修

| # | 风险 | 处置 |
|---|---|---|
| 11 | /migrate 不在 dev proxy | 前端小债批次（Deferred） |
| 12 | .env.example 缺 9 变量 | **T002（Next）** |
| 13 | 前端死代码群 | 前端小债批次（Deferred） |
| 14 | 原生 confirm/prompt×2 | 前端小债批次（Deferred） |
| 15 | 无 404 + 无 ErrorBoundary | 前端小债批次（Deferred） |
| 16 | token localStorage + CSP unsafe-inline | Deferred（单用户自托管定位，接受） |
| 17 | Python e2e 零 CI | 杂项批次 |
| 18 | README 两处口径过时 | **T003（Next）** |
| 19 | verify 脚本端口撞车 + zztest_m10 | 杂项批次 |
| 20 | OpenAPI 覆盖不全且无鉴权 | 杂项批次 |
| 21 | login_throttle 进程内 / client_ip 信 XFF | Deferred（记录权衡即可） |
| 22 | 实体归并 LIMIT 1 无序 / 关系回溯无水位 | Deferred（随敏感批次顺手） |
