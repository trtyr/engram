# Topic: 检索智能

覆盖 R1/R2/R3/R6/R8/R9/R13。核心问题：检索是「平台即工具」的消费入口，现在分散 + 召回有损。

## 现状盘点

- memory 域：`search_atoms` / `search_scenarios` 单 SQL 内 FTS + ANN + RRF（`RRF_K=60`），中文 jieba 预分词。
- knowledge 域：chunk 有 `tsv` + `embedding`，但检索走 `knowledge/mod.rs` 自己的查询，与 memory 不共享 RRF 逻辑。
- wiki 域：`WikiService::search` 纯 FTS（`ts_rank` 排序），向量通道已存但检索未用。
- **三域检索零复用**：RRF 融合只存在于 memory 的 atoms/scenarios，wiki/knowledge 各自为政。

## R1 跨域统一检索

- 目标：一个统一入口，query → 三域并行检索 → RRF 融合 → 统一 `SearchHit`（带域标签）。
- 价值：用户/Agent 问一个问题，同时命中「记忆里的偏好 + 知识库文档 + wiki 页面」，而不是调三个接口拼结果。
- 实现方向：`search` crate 加 `search_unified`，复用现有 `rrf.rs` 融合三域命中；`core` 加编排门面。

## R2 tsquery 召回优化

- 现状：`tsv_query` 把分词结果 `join(" & ")`，全部 token 必须命中才返回，查询越长召回越差。
- 目标：短查询 AND，长查询允许 OR 兜底（或 top-N token 用 `|`），命中数下限可调。
- 依据：评论区 huachen-wang「index 有损瓶颈，用并集修」的同一逻辑——只增不减。

## R3 context_pack L1 相关性

- 现状：`context_pack` 的 L1 补充是 `ORDER BY hit_count DESC, confidence DESC`，跟当前 query 无关，等于「全局最热原子」，不是「与本次对话相关的原子」。
- 目标：L1 也走 `search_atoms(query, qv, ...)`，与 L2 同源相关。
- 价值：冷启动上下文包更贴合当前对话，减少无关记忆占预算。

## R6 LLM rerank

- 现状：RRF 融合后按分数截断，无重排。
- 目标：top-k（如 20）内用 LLM 重排（可选开关，成本可控），对齐 qmd 的「BM25/向量 + LLM 重排」。
- 注意：单用户场景默认关，成本敏感；做成可选增强。

## R8 命中反馈

- 现状：`SearchHit` 有 `hit_count` 字段，`atoms.hit_count` 有索引价值，但检索从不 `hit_count++`。
- 目标：命中即累加，让「常被命中的记忆」自然浮到 context_pack 前列（强化式排序）。

## R9 chunk 上下文扩展

- 现状：chunk 检索只返回 chunk 自身 content，缺父文档标题与相邻 chunk。
- 目标：返回时带 `document_title` + 可选相邻 chunk（前后各 1），提升 Agent 理解上下文。

## R13 Read Sources Only

- 现状：无「仅原文」开关。
- 目标：检索加 `mode: sources_only`，跳过 wiki 综合/蒸馏产物，只返回原始文档 chunk。
- 依据：llm_wiki 的安全阀，对抗「幻觉固化进 wiki」；单用户自查也用得上。
