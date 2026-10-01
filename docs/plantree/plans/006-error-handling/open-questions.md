# P006 Open Questions

## 未决

### Q001 · 替换策略（T002 前提）
- **A · 渐进下沉**（倾向）：新模型从边界层（MCP/HTTP 响应格式）先立，内部 crate 现有 error enum 按 crate 分批迁移——每批独立可合、workspace 永远绿；周期长。
- **B · 一次性统一**：全 crate 一步到位换 EngramError——彻底但巨型 PR，回归风险高，与「小步提交」纪律冲突。

### Q002 · 错误码格式
- **A · 域前缀字符串**（倾向）：如 `LLM-TIMEOUT` / `WIKI-PAGE-NOT-FOUND` / `AUTH-401`——人读可猜、agent 可判断；注册表防重。
- **B · 数字码**：如 `ENG-1004`——紧凑但需查表，agent 不可猜。

### Q003 · 对外脱敏程度（T004 前提）
- 单用户自托管系统、调用方=owner 本人 + 自己的 agent——内部 bug 细节（crate/函数级位置）是否也对外给？
- 倾向：给全（owner 有权看全部，脱敏反而碍调试）；仅密钥值/token 永不出现（红线不变）。

## 已决归档

（暂无）
