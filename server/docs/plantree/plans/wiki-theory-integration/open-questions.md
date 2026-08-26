# Open Questions

1. **gap 是否转实现计划？** `gap-analysis.md` 点出的 G1（并发去重）/ G2（pin 存活）是代码改动，超出本计划「纯文档」范围。是否另起一个实现计划处理？还是仅存档供未来参考？
2. **G6 copied-state 规范**：是否要把「文档不记会变的值」这条吸收进 `docs/conventions.md`？
3. **定位确认**：overview 称「单用户平台」，但理论谱系里 TencentDB 是团队级。若未来要多用户/多 Agent 并发，G1 优先级会上升——当前是否需要预留？
