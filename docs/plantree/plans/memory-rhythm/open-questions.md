# Open Questions — memory-rhythm

## 1. cron 的语义层边界 ✅ 已关（D-002）

**选层 B**：维护性 + 记录性兜底（pending 会话超 N 小时由 cron 扫走）。不依赖 AI 自觉的保险。

## 2. cron 宿主 ✅ 已关（D-002）

**选外部 crontab 打 API**（非内置 scheduler）。灵活，多一个活动部件但 server 侧更轻。

## 4. 静默时段的语义 ✅ 已关（随外部宿主消解）

cron 住外部 → 静默时段就是 crontab 的时间表达式本身，server 不控制也不判定。
设置页只观察心跳是否逾期，不提供运行时间配置。
