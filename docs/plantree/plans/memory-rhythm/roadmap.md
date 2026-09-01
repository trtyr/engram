# Roadmap — memory-rhythm

## In Progress

- [ ] **R-4 验收**：全门禁（cargo 140 / vitest 35 / build / lint）+ 推送盯 CI+e2e 双绿（regression-final）

## Done

- [x] **R-0 语义拍板**：D-002 外部 cron + 层 A+B（见 decisions/）
- [x] **R-1 cron 节律落地**：`chain::trigger` cron 通道（consolidate 日桶幂等）+ heartbeat/status 端点 + 触发源标记（commit 775aea2）
- [x] **R-2 冲突增量防御**：full 日桶幂等（冲突矩阵 A）；一次性栈活体复现 3 场景（evidence/conflict-matrix-live.md）
- [x] **R-3 Settings 配置页**：节律 tab（心跳监控/积压年龄/安装向导/事件流，commit c1f877f）

## Deferred

- [ ] harness 侧 pi extension 钩子（open-questions #3）
- [ ] conflict-matrix B 项（防抖 starvation 上限）、D 项（并发上限）——上线后观察，暂不写场景
