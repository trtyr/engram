# Phase 2 — 记忆域（Chat Memory L0–L3）

**目标**：平台核心价值闭环——写入会话，蒸馏出分层记忆，画像可溯源，检索可用。这是「记忆平台」的本体。

## 前置

Phase 1。

## 交付物

### L0 写入与触发

- [ ] `POST /memory/sessions`（多轮结构：speaker/content/ts 数组）+ sha 查重
- [ ] 触发策略：debounce 聚合（默认 30s 可配）/ 手动 `POST /memory/distill`
- [ ] L0 列表/详情 API（含该会话蒸馏产物反向链接）

### 蒸馏链（distill crate）

- [ ] extract_atoms job：P_EXTRACT_v1，批量读新 L0 → 候选 atoms（含 kind/confidence/source_refs）
- [ ] arbitrate job：候选 × 相似既有（向量预筛 + LLM 仲裁）→ 新增/去重/矛盾 supersede
- [ ] organize_scenarios job：未归组 L1 → 新建/更新 L2（增量，不全量重建）
- [ ] distill_persona job：变动 L2 → L3 aspect 新版本（版本化 + evidence_refs 链）
- [ ] consolidate job：近重复合并、stale 降权、L2 重聚类；每周 due_at 调度 + 手动
- [ ] 每步产物记 prompt_version/model/usage；单链 token 预算熔断
- [ ] 低置信度 atom → needs_review 标记（不阻塞）

### 检索与上下文

- [ ] L1/L2/L3 写入时 embedding + 预分词 tsvector
- [ ] `POST /memory/search`（layers 过滤 + 预算）· `GET /memory/context`（冷启动包：L3 全量 + 相关 L2 + top L1）
- [ ] 中文样例集（≥10 条真实中文记忆写入→检索命中）集成测试

### 管理与治理 API

- [ ] atoms 列表/过滤/手工新增/编辑/supersede；scenarios 列表/详情
- [ ] persona 视图 + 版本历史（diff 数据结构）+ aspect 版本回滚
- [ ] L0 擦除（级联 source_refs 失效标记）

## 出口标准

1. e2e 脚本（真 LLM，evidence 留档）：写入 3 段中文会话 → 蒸馏链自动跑完 → L1 出现正确 kind 的 atoms → 矛盾语句使旧 atom superseded → 画像 aspect 更新且 history 可 diff
2. `GET /memory/context` 对冷启动查询返回预算内的三层结构包，引用链完整（L3→L2→L1→L0 逐级可回溯）
3. mock provider 单测覆盖：抽取解析失败重试、仲裁三分支、persona 版本化、consolidate 合并
4. 全域 API 进入 OpenAPI 快照；Phase 0/1 出口标准依然全绿

## 关联

- 设计：[memory-model](../topics/memory-model.md)、[distill-pipeline](../topics/distill-pipeline.md)、[search](../topics/search.md)
- 风险：R2（提示词质量——本阶段建 needs_review + 版本回滚两道闸）、R5
