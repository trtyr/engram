# 已实测验证记录

## 1. session-write → 蒸馏自动加工（2026-09-02）

**结论**：AI 只用 `session-write` 写用户口述，蒸馏自动完成实体+关系+原子的完整加工。

**过程**：
- 写 3 轮用户口述（长亭团队/数据集团项目/谢乐关系）→ `session-write --distill auto`
- 30s 防抖后 extract_atoms 自动跑

**结果**：
- 抽出 10 条候选原子：3 条 promoted（新信息）+ 7 条 duplicates（判重归档）
- 关系 22 条 source=distill
- source_refs 完整指向会话 `01a06029`
- organize_scenarios + distill_persona 全跑（identity v3 + skills v4）

## 2. correction 走会话（2026-09-02）

**结论**：correction 不需要 AI 直写原子，写"纠正对话"即可，蒸馏自动完成整条链。

**过程**：写会话"纠正一下：之前说'手上在背的项目'其实是'手上在跟的项目'" → distill

**结果**：
- 旧原子 `01a05844`"在背"→ status=`superseded`，superseded_by 指向新原子 `01a06034`
- 新原子"纠正为'在跟'"→ active
- arbitrate 的 superseded 数组记录 `{new, old}` 取代对
- 画像 identity v4 同步更新

## 3. 敏感保护（P3，早前验证）

- 2 条 sensitive 原子（鹿鞭地黄丸 + 大连家庭地址）正确锁定
- 体检：隐私词命中但未锁 = 0 遗漏

## 4. 用户记忆原子层体检（2026-09-02）

- 敏感遗漏 0、判重链无断链、矛盾残留 0、过期残留 0
- 发现：41 条无 source_refs（直写残留）、3 条 event/decision 缺 occurred_at、FDE 误译

> 刁钻用例实测结果（19 通过 + 2 bug）已同步到 `/tmp/agent-memory-findings-2026-09-02.md`（给开发的投递文档），此处不重复。
