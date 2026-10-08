# 风险热点速览

> 详情权威：档案「风险与债（四维排序）」01a0e6f6-04d2-7b03-b128-29863af0e008。此处只做工作提醒。

## P0 会咬人

| 风险 | 位置 | 一句话 |
|---|---|---|
| 生产反代无 IaC | 仓库外 | EdgeOne/Caddy 配置零留痕，不可审计不可重现 |

## P1 语义债（需要产品拍板）

| 风险 | 状态 |
|---|---|
| 敏感语义分裂（检索放开 vs 蒸馏/画像/时间线仍排除） | **已拍板方向**：统一放开（决策 001），边界待 Q001 |
| atoms/scenarios 无 prompt_version 列（L0-L2 归因断链） | 待拍板是否补列 |
| HTTP/MCP 读动作口径不对称 | 待拍板收紧方向 |

## P2 规模

10k 页 wiki：pages_list 4.5s / graph 3.1s（results/stress-10k.json）——目录树与图谱 API 是首要天花板；前端全量拉取+无缓存层叠加恶化。

## P3 便宜修

/migrate 不在 dev proxy · .env.example 缺 9 变量 · 前端死代码群（react-query/setBase/withLib/uploadArtifact×2/revive_entity）· 原生 confirm/prompt×2 · 无 404/ErrorBoundary · README 两处口径过时 · e2e 零 CI 接线 · verify 脚本 PG 端口撞车 · zztest_m10 硬编码 8080 · OpenAPI 覆盖不全且无鉴权 · /metrics public 无鉴权 · login_throttle 进程内 · 子串实体归并无序 LIMIT 1 · 关系回溯 weight 无水位。

完整 22 项四维排序见档案风险篇。
