# Roadmap — memory-rhythm

## Next

- [ ] **R-0 语义拍板**（阻塞后续全部）：open-questions #1（cron 的记录性 vs 维护性边界）、#2（cron 宿主：server 内置 vs 外部打 API）
- [ ] **R-1 cron 节律落地**：定时任务注册 + 幂等 + 与现有 debounce/队列合一
- [ ] **R-2 冲突增量防御**：conflict-matrix 中「需要增量」的项
- [ ] **R-3 Settings 配置页**：节律配置面（开关/周期/静默时段/最近节律事件）
- [ ] **R-4 验收**：冲突场景测试矩阵跑通（AI×cron 并发、cron 中途 purge、provider 故障窗口）

## Deferred

- [ ] harness 侧 pi extension 钩子（session_start 注入 / turn_end append / shutdown flush）——与 cron 是互补关系，见 open-questions #3

## Done

（尚无）
