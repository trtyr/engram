# Open Questions — memory-rhythm

## 1. cron 的语义层边界 ✅ 已关（D-002）

**选层 B**：维护性 + 记录性兜底（pending 会话超 N 小时由 cron 扫走）。不依赖 AI 自觉的保险。

## 2. cron 宿主 ✅ 已关（D-002）

**选外部 crontab 打 API**（非内置 scheduler）。灵活，多一个活动部件但 server 侧更轻。

## 3. 与 pi extension 钩子的关系（不阻塞，仍开）

harness 钩子（session_start 注入 / turn_end append / shutdown flush）在 Deferred。
钩子管「会话内实时」，cron 管「跨会话周期兜底」，互补不冲突；cron 已上线，
钩子做不做取决于用户要不要「开场自动注入」那一层实时性。

## 4. 静默时段的语义 ✅ 已关（随外部宿主消解）

cron 住外部 → 静默时段就是 crontab 的时间表达式本身，server 不控制也不判定。
设置页只观察心跳是否逾期，不提供运行时间配置。
