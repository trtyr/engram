# Topic — 记忆模型（L0–L3）

借鉴 TDAM 分层 + hermes 分类学，融合为单用户长期记忆模型。

## 分层

| 层 | 实体 | 内容 | 更新方式 | 检索角色 |
|---|---|---|---|---|
| L0 | raw_sessions | 原始会话（多轮，含 speaker/时间） | 只写不改（不可变） | 精确回溯、溯源终点 |
| L1 | atoms | 原子事实/偏好/事件 | 蒸馏产生；可 supersede | 精确事实召回 |
| L2 | scenarios | 场景/主题知识块（含摘要+正文） | 蒸馏增量更新 | 快速恢复工作上下文 |
| L3 | persona_aspects | 用户画像（按 aspect 分面） | 蒸馏增量 + 版本化 | 冷启动注入 |

## L1 kind 分类（融合两派）

| kind | 含义 | 例子 |
|---|---|---|
| preference | 用户偏好 | 「回答要简短」 |
| fact | 稳定事实 | 「住上海，用 Mac」 |
| decision | 已定决策 | 「后端选 Rust 不选 Go」 |
| event | 事件 | 「10 月上了线 v2」 |
| insight | 洞察 | 「Ta 的项目都用 pnpm」 |
| correction | 用户纠正 | 「不要用 emoji 回复」 |
| failure | 失败教训 | 「直接 rm -rf 被骂了」 |
| convention | 约定 | 「提交信息用中文」 |

每个 atom：content（一句话）、confidence、source_refs（指向 L0 的 id+span）、status、embedding、tsv。

## 矛盾消解

新 atom 入库前与既有 active atoms 比对（embedding 相似度阈值 → LLM 仲裁）：

- **重复** → 丢弃新条，旧条 confidence+hit_count 递增
- **矛盾** → 旧条 `status=superseded, superseded_by=new`；历史保留可查
- **新知** → 直接入库

检索默认只返回 active。这是画像不漂移的关键机制。

## L3 画像分面（aspect）

`identity / preferences / skills / constraints / communication_style / goals / routines`

- 每个 aspect 一条内容（自然语言段落），带 evidence_refs → L2/L1/L0 引用链
- 每次更新产生新版本；`GET /memory/persona/history` 可看 diff——画像里每句话可溯源
- 删除权：L0 可整条擦除（连带 source_refs 失效标记），GDPR 式删除语义

## 检索默认策略

`GET /memory/context`（AI 冷启动）：L3 全量（很小）→ 相关 L2 topN → 按预算补 L1。
`POST /memory/search`：全部命中层并 RRF 融合；`layers` 参数可限定。

## Consolidation（定期整理）

- 近重复合并、stale 降权（长期未命中 + 低 confidence）
- L2 重新聚类（增量，不全量重建）
- 频率：手动 + 每周定时；产物也是 job，全程可观测
