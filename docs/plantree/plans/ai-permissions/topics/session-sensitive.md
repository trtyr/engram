# 会话级敏感标记

写会话时声明 sensitive，蒸馏产物自动继承，消除"AI 等异步蒸馏完再手动 atom-patch --sensitive"的泄露窗口。

## 问题

当前流程：AI 写会话 → 30s 防抖异步蒸馏 → 蒸馏产物落库 → AI 再手动 `atom-patch --sensitive` 锁敏感产物。

**窗口**：从"产物落库"到"AI 手动锁"之间，敏感内容已在库中未锁，可能被 search/context 检索到。

## 方案

`session-write` 支持声明 sensitive（会话级或轮次级），蒸馏产物自动 `sensitive=true`：

- 会话级：整段会话标 sensitive → 所有产物继承
- 轮次级：只标某轮 user turn → 该轮产物继承（粒度待定，见 open-questions）

## 验证

见 test-plan.md T3：产物自动 sensitive → search 默认隐身 + --reveal 可见 + context_pack 排除。

## 关联

- 若落地，`atom-patch --sensitive` 可进一步收回（AI 在写会话时就声明敏感）
- 与 P3 敏感保护三链（检索隐身/context 排除/export 排除）衔接
