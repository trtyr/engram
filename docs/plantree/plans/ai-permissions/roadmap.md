# Roadmap

## Done（已实测验证）

- **session-write → 蒸馏自动加工**（2026-09-02 实测）：3 轮用户口述 → 抽出 10 条原子 + 22 条关系（source=distill）+ 画像更新；判重正确（7 条重复归档）
- **correction 走会话**（2026-09-02 实测）：写"纠正对话" → 蒸馏自动归档旧原子 + 生成新原子 + superseded_by 取代链 + 画像 identity v4 更新
- **敏感保护三链**（P3，早前验证）：search/context 默认隐身，--reveal 可见；2 条 sensitive 原子（鹿鞭地黄丸 + 家庭地址）正确锁定，0 遗漏

## Next（待开发，按优先级）

1. **AI 权限收窄**：收回 `atom-add` / `entity-add` / `entity-rel-add` / `entity-attach`，只留会话写入 + 保护类（sensitive/status）+ distill 触发
2. **文件批量导入为会话**：session-write 的批量版，上传文件（JSONL/文本）→ 自动解析成会话 turns → 蒸馏
3. **会话级敏感标记**：写会话时声明 sensitive，蒸馏产物自动继承（消除"异步蒸馏完再手动锁"的泄露窗口）
4. **检索时间范围过滤 + 过期降权**（旧 roadmap 项）

## Deferred

- 清理 41 条无溯源原子（直写残留，数据迁移需评估影响）
- FDE 误译修正（画像把 Forward Deployed Engineer 译成"前端部署工程师"）
- 关系 source 混存（38 manual + 22 distill）去重
