# memory-rhythm（记忆节律）

记忆的写入与维护节律：AI 主动请求 + cron 定时两条腿，冲突防御，以及设置页配置面。

## Scope

- **In**：双节律模型（AI 主动 / cron 定时）、AI×cron 冲突矩阵、cron 宿主选型、Settings 配置页（开关/周期/静默时段）、节律可观测性（设置页看到"谁在记、记了什么"）
- **Out**：蒸馏算法本身（已有）、编辑分权（已有）、harness 侧 pi extension 钩子（相邻但独立，见 open-questions #3）

## Authority

用户方向（2026-09-01 傍晚拍板）：「以后会有两个方式，一个是 AI 主动请求记录，一个是 cron 定时。要考虑到极端情况，比如 AI 和 cron 在某一时刻冲突。设置那里需要加一个相关的配置页面。」

## File Map

| 文件 | 角色 |
|---|---|
| [roadmap.md](roadmap.md) | 阶段状态（Done / In Progress / Next / Deferred） |
| [topics/dual-rhythm.md](topics/dual-rhythm.md) | 双节律方案胶囊：两条腿各自的语义与边界 |
| [topics/conflict-matrix.md](topics/conflict-matrix.md) | 冲突矩阵：已内建防御 vs 需要增量 |
| [topics/settings-page.md](topics/settings-page.md) | 设置页配置面方案 |
| [open-questions.md](open-questions.md) | 未决问题 |
| [decisions/](decisions/) | 决策记录 |

## Reading Path

1. 本 README（scope）
2. topics/dual-rhythm.md（两条腿是什么）
3. topics/conflict-matrix.md（哪些极端情况已经防住、哪些要新建）
4. open-questions.md（动手前要拍的板）
