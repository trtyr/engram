# Topic — Wiki 引擎

Karpathy 模式（llm_wiki 实践验证）：原料不可变，LLM 增量维护 wiki，人负责纠偏。数据库为真相源，页面以 Markdown+frontmatter 存储。

## 结构

```text
wiki_sources   # 原料（不可变副本，独立于 knowledge 的 uploads）
wiki_pages     # LLM 生成/维护的页面
wiki_links     # 图边：wikilink 边 + source-overlap 边
```

### 特殊页（系统维护）

| 页 | 职责 |
|---|---|
| index | 内容目录：全部页面 + 一句话摘要（每次 ingest 后更新） |
| log | 操作日志：append-only，每行可解析（时间、动作、source、结果） |
| overview | 全局摘要，每次 ingest 后重生成 |

### 页面类型 page_type

`entity（实体）/ concept（概念）/ source（源摘要）/ synthesis（跨源综合）/ comparison（对比）/ overview / index / log`

### frontmatter schema

```yaml
title, page_type, sources: [wiki_source_id], updated_at, version,
origin: llm | human,   # 最近一次由谁写入
```

## 两步 ingest（质量关键，llm_wiki 验证有效）

1. **analysis job**：LLM 读 source 全文 + index.md + 相关既有页 → 产出结构化分析：实体/概念清单、与现有页的关联、矛盾点、结构建议
2. **generation job**：LLM 按分析产出 → 新建/更新 entity/concept/source 页（带 `[[wikilink]]`）→ 更新 index/log/overview → 新页嵌入入向量库 → 增量更新 wiki_links

- sha256 缓存：source 未变则 ingest 直接跳过
- 每个 wiki 页 frontmatter 记录 `sources[]`——删除级联与溯源的基础（借鉴 llm_wiki 级联删除）

## 人工编辑与覆盖保护

- `PUT /wiki/pages/:slug` 人工编辑 → `origin: human`，版本 +1
- ingest 若要覆盖 `origin: human` 的页 → 不直接覆盖，生成 **proposal**（diff 形式）进人审队列，UI 确认后合入
- 人审队列复用 needs_review 机制

## Lint（健康检查）

`POST /wiki/lint` 返回报告：

| 规则 | 检测 |
|---|---|
| dead_link | 指向不存在页的 [[wikilink]] |
| orphan | 无入链且非系统页 |
| stale_source | wiki_source sha 变了但页面未重新 ingest |
| broken_frontmatter | schema 校验失败 |
| duplicate_entity | 同名实体多页 |

lint 只报告不改写；修复动作由人或 AI 显式触发。

## 检索与图

- 页面检索走统一 search（embedding + FTS）
- `GET /wiki/graph` 返回节点（页面）+ 边（wikilink/source-overlap），前端 sigma.js 渲染
