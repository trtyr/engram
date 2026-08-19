# Phase 4 — Wiki 域

**目标**：Karpathy 模式完整落地——文档进、wiki 出、链接图活、lint 有效、人工纠偏有版本保护。

## 前置

Phase 1（建议在 Phase 3 后做，摄取复用其解析能力；亦可并行，解析独立实现）。

## 交付物

### 原料与页面模型

- [ ] wiki_sources：ingest 时从 knowledge 文档复制独立不可变副本（或直接上传）
- [ ] wiki_pages 全 page_type + frontmatter schema 校验；`[[wikilink]]` 解析器（含中文 slug）
- [ ] index/log/overview 系统页维护（log 行格式可解析）

### 两步 ingest

- [ ] analysis job：source + index.md + 相关页 → 结构化分析（实体/概念/关联/矛盾/结构建议）
- [ ] generation job：按分析产出/更新页面（frontmatter sources[] + wikilink）→ 更新 index/log/overview → 新页 embedding → wiki_links 增量
- [ ] sha256 缓存跳过；中文源中文产出（Q5 约定）
- [ ] 覆盖保护：`origin: human` 页面只产出 proposal diff 进人审队列，不直接覆盖

### API 与治理

- [ ] pages 列表/详情/人工编辑（版本化）；graph 端点（节点+边）
- [ ] `POST /wiki/lint`：dead_link / orphan / stale_source / broken_frontmatter / duplicate_entity
- [ ] 检索接入统一 search（wiki scope）

## 出口标准

1. e2e（evidence 留档）：两篇相关中文文档先后 ingest → 产出 entity/concept/source 页、`[[wikilink]]` 互链、index/log 正确追加；第二篇引用第一篇实体时正确更新既有页而非重复建页
2. 同一 source 重复 ingest → 秒跳过（事件流水验证）
3. 人工编辑某页后再次 ingest 触及该页 → 产生 proposal 而非覆盖，人审合入后版本连续
4. lint 对人工注入的死链/孤儿页全部报出，无误报样例
5. mock LLM 单测：两步 prompt 解析失败重试、frontmatter 校验拒绝坏输出；Phase 0/1 出口标准依然全绿

## 关联

- 设计：[wiki-engine](../topics/wiki-engine.md)
- 风险：R2、R6
