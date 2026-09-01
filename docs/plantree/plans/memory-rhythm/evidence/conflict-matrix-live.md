# conflict-matrix live evidence（2026-09-01）

一次性栈（`am_conflict_*` + 独立端口 + 自动拆）活体复现三个关键冲突场景。
规则遵守 P11：绝不碰生产库；合法参数只对假数据。

## A. cron full 双入队 → consolidate 日桶幂等

- 同日内两次 `POST /memory/distill {"full":true,"via":"cron"}`
- consolidate 返回**同一 job id**（`01a05ca4-1b42-...`）——网络 retry 风暴 / crontab 双行不会重复烧 LLM
- extract 各自新 job——扫 pending 是兜底本意，永不去重
- 触发源标记进 payload（`reason=cron`、`triggered_by`）

## B. cron 蒸馏完成后 AI append → 400 教学文案

- session 蒸馏到 `done` 后 `POST /memory/sessions/{id}/append`
- 400 + 文案「会话已蒸馏（done），不可追加——请开新会话」
- 错误三问齐全：发生了什么 / 为什么（已蒸馏）/ 下一步（开新会话）

## C. 心跳逾期判定

- 心跳落库 → SQL 回拨 `created_at - 3 days`
- `GET /memory/rhythm/status` 报 3 天前的心跳
- 设置页按「期望周期 × 1.5」阈值渲染逾期（UI 渲染由 vitest + task-2 活体验证覆盖）

## 未覆盖项（已知，非本次阻断）

- conflict-matrix 的 B 项（防抖 starvation 上限）与 D 项（并发上限）属于
  cron 上线后观察项，未在本次写场景——见 roadmap R-4 收尾
