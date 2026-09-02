# ai-permissions（AI 权限模型收窄）

AI 消费者的写权限边界：**AI 只写会话（记录者），系统蒸馏加工（加工者）**。收回 AI 直写原子/实体/关系的权限，补齐文件导入与会话级敏感标记两个配套能力。

## Scope

- **In**：AI 写权限收窄（会话-only）、文件批量导入为会话、会话级敏感标记、correction 走会话（已验证）、检索增强（时间范围过滤 + 过期降权）
- **Out**：蒸馏算法本身（已有）、编辑分权（编辑=用户权力，已有）、pi extension 钩子（宿主侧，非本系统能力）

## Authority

用户方向（2026-09-02 拍板）：「是不是不应该给你开放权限？应该只给你开放会话的权限。你只能去编辑，但是不能去添加。」

实测支撑（2026-09-02）：AI 只用 `session-write`，蒸馏自动抽出实体+关系+原子、自动判重、自动 correction、自动更新画像——**直写加工是多余的，还断溯源**。

## File Map

| 文件 | 角色 |
|---|---|
| [roadmap.md](roadmap.md) | 阶段状态（Done / Next / Deferred） |
| [test-plan.md](test-plan.md) | 详细测试计划（核心，T1–T6） |
| [topics/permission-matrix.md](topics/permission-matrix.md) | 权限矩阵方案胶囊 + 403 文案 |
| [topics/file-import.md](topics/file-import.md) | 文件批量导入方案胶囊 |
| [topics/session-sensitive.md](topics/session-sensitive.md) | 会话级敏感标记方案胶囊 |
| [evidence/live-verifications.md](evidence/live-verifications.md) | 已实测验证记录 |
| [open-questions.md](open-questions.md) | 未决问题 |

## Reading Path

1. 本 README（scope）
2. [test-plan.md](test-plan.md)（测试全景，先看这个）
3. [topics/](topics/)（各方案胶囊）
4. [open-questions.md](open-questions.md)（动手前要拍的板）
