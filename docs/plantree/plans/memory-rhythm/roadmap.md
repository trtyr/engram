# Roadmap — memory-rhythm

## Done

- [x] **R-0 语义拍板**：D-002 外部 cron + 层 A+B（见 decisions/）
- [x] **R-1 cron 节律落地**：`chain::trigger` cron 通道（consolidate 日桶幂等）+ heartbeat/status 端点 + 触发源标记（commit 775aea2）
- [x] **R-2 冲突增量防御**：full 日桶幂等（冲突矩阵 A）；一次性栈活体复现 3 场景（evidence/conflict-matrix-live.md）
- [x] **R-3 Settings 配置页**：节律 tab（心跳监控/积压年龄/安装向导/事件流，commit c1f877f）
- [x] **R-4 验收**：全门禁绿 + 推送盯 CI+e2e 双绿（775aea2..6c495f7）
- [x] **R-5 分权根修**（测试方 seq 三连实锤后补）：via:cron 与 heartbeat 改 cron scope 专属，杜绝 AI 伪造（commit d7a8345 + 生产栈重启探针 + 测试方正反矩阵闭合）

## Deferred

- [ ] harness 侧 pi extension 钩子（open-questions #3）
- [ ] conflict-matrix B 项（防抖 starvation 上限）、D 项（并发上限）——上线后观察，暂不写场景
