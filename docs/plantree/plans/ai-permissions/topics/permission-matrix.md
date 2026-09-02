# 权限矩阵方案

AI 消费者权限收窄：**只留会话写入 + 保护/纠错，收回直写加工**。

## 权限矩阵

| 操作 | 收窄前 | 收窄后 | 理由 |
|---|---|---|---|
| `session-write` / `append` / `void` | ✅ | ✅ | 记录原料，AI 本职 |
| `distill` / `--full` | ✅ | ✅ | 写完让系统消化 |
| `atom-patch --sensitive` | ✅ | ✅ | 保护（隐私锁），非加工 |
| `atom-patch --status archived` | ✅ | ✅ | 保护/纠错 |
| `atom-add` | ✅ | ❌ 403 | 生产成品，是蒸馏的活 |
| `entity-add` | ✅ | ❌ 403 | 实体由蒸馏抽取 |
| `entity-rel-add` | ✅ | ❌ 403 | 关系由蒸馏抽取 |
| `entity-attach` | ✅ | ❌ 403 | 挂原子由蒸馏完成 |
| `atom-patch --superseded-by` | ✅ | ❌ 403 | correction 走会话，蒸馏自动取代 |

## 403 文案（三问齐全）

每个收回的操作，403 body 必须回答"发生了什么 / 为什么 / 下一步正确姿势"：

- `atom-add` →「原子由会话蒸馏产生——AI 记录对话（session-write）即可，蒸馏自动抽取；人工直写断溯源且绕过判重」
- `entity-add` →「实体由蒸馏从会话中抽取，AI 写会话即可」
- `entity-rel-add` →「关系由蒸馏抽取（source=distill），AI 写会话即可」
- `entity-attach` →「原子-实体挂接由蒸馏自动完成」

## 实现要点

- 沿用 `erase_session` 同款模式：PATCH/POST handler 内 Principal 二次校验，无新中间件
- 分权落点：按"改写 vs 保护/追加"划线，与 content/kind/confidence 编辑分权同一套思路
- 关键不变量：**凡必须不发生的事不交给模型**——直写加工是"AI 越位当蒸馏器"，确定性拒绝

## 配套依赖

- correction 走会话（已实测 ✅）→ 收回 superseded-by 的前提成立
- 会话级敏感标记（topics/session-sensitive.md）→ 收回 atom-patch --sensitive 的前提（否则 AI 仍要等蒸馏完手动锁）
