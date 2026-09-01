# Open Questions — memory-rhythm

## 1. cron 的语义层边界（阻塞 R-1）

维护性（层 A：定时蒸馏/收敛/退休，无歧义）与记录性（层 B：定时产生/搬运记忆）怎么切？

- 选项 a：只做层 A——cron 纯维护，记录性全靠 AI 主动 + 未来 pi 钩子
- 选项 b：层 A + 层 B 的最小子集（如"pending 会话超 N 小时未蒸馏则 cron 兜底触发"）

倾向 a（最小惊讶），但用户说「cron 定时记录」可能心里有 b。**待用户拍板。**

## 2. cron 宿主（阻塞 R-1）

- 选项 a：**server 内置 scheduler**——jobs 表已有 due 轮询，注册周期任务 = 到点自动 enqueue（幂等键防重）。零新组件，重启自动续跑。
- 选项 b：外部 cron 打 API——灵活但多一个活动部件，且 API 鉴权要开洞。

倾向 a。**待确认。**

## 3. 与 pi extension 钩子的关系（不阻塞）

harness 钩子（session_start 注入/turn_end append/shutdown flush）在 Deferred。
若钩子做了，层 B 的记录性 cron 大部分被覆盖 → 支持 #1 选 a 的另一个理由。
但钩子管的是"会话内实时"，cron 管"跨会话周期"，长期共存不冲突。

## 4. 静默时段的语义

静默窗口内 due 的任务是顺延还是跳过？（顺延 = 窗口结束后补跑；跳过 = 等下个周期）
倾向顺延（维护性任务漏跑一天没意义）。小决策，实施时定即可。
