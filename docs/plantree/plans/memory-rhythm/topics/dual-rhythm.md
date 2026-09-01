# 双节律方案（dual-rhythm）

两条腿各自管什么。核心原则：**AI 管对话实时性，cron 管周期维护性；两者永不直接对话，只通过 jobs 队列与幂等键交错。**

## 腿 1：AI 主动请求记录（已上线）

- 语义：AI consumer 在对话中调 `session-write` / `session-append`（30s 防抖桶自动触发 extract）
- 时机由 AI 判断（听到值得记的就写）；纠错走 correction 双条留痕
- 已有完整 API 面 + skill（memory.py）+ 分权（amk_ scope）

## 腿 2：cron 定时（待建）

两个候选语义层（open-questions #1 待拍板）：

### 层 A：维护性 cron（无歧义，可直接做）

定时跑**已有**的周期任务，不产生新记忆，只维护存量：

| 任务 | 周期候选 | 现状 |
|---|---|---|
| full 蒸馏（consolidate + 实体档案 + 退休检查） | 每日一次 | 手动 `distill --full`，无定时 |
| 快照收敛（organize converge pass） | 每日一次 | 事件驱动（归档/敏感触发），无兜底定时 |
| 分面退休检查（R3 7 天年龄） | 每周一次 | 随 full consolidate 附带，无独立定时 |

### 层 B：记录性 cron（语义待定）

定时**产生/搬运**记忆的兜底——如 harness 侧定时 flush、定时把"攒着的待写内容"落库。
这一层与 pi extension 钩子（Deferred）高度重叠，需要先拍板边界再动。

## 为什么不直接让 AI 全权负责

用户原话的北极星：「真记性不需要主人提醒自己在记」。AI 主动是理想态，
但 AI 忘了调/会话崩了/多 agent 场景下总有漏——cron 是**不依赖任何 AI 自觉性**的兜底线。
