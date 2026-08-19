# Topic — 知识摄取（Knowledge）

文档/URL → 解析 → 分块 → 嵌入 → 可检索。服务于 AI 的「外部资料长期记忆」。

## 支持格式（第一版全做到）

| 格式 | 解析方案 |
|---|---|
| md / txt | 直读 |
| pdf | pdf-extract（Rust） |
| docx | docx-rs |
| html | htmlq/readability 式正文抽取 |
| URL | 服务端抓取（SSRF 防护，见下）→ 同 html |

> xlsx/pptx/epub 记入 ideas，不进第一版（控制范围）。

## 管道（每步一个 job，状态机推进）

```text
pending → parsing → chunking → embedding → ready
                ↘ failed（error + 可重试判定）
```

1. **parsing**：提取纯文本 + 标题结构；原始文件存 `data/uploads/`，sha256 入库（同文件秒级去重返回已有 doc）
2. **chunking**：结构感知分块（按标题优先，目标 512–1024 token，相邻重叠 ~15%）；块内容不再清洗
3. **embedding**：按路由规则调 embedding 模型；批量并行（并发上限可配）；失败单块重试

## URL 抓取安全（SSRF 防护）

- 解析 DNS 后校验：拒绝私网/环回/链路本地/保留地址（含解析出的全部 A/AAAA）
- 重定向跟随 ≤3 且每跳重新校验；仅 http/https；响应体大小上限（默认 20MB）；超时 30s
- UA 标识 `agent-memory/1.0`

## 检索

`POST /knowledge/search {query, budget}` → FTS + 向量 RRF 融合 → 结果带 `document_id + title + chunk 引用 + 高亮片段`。

## 删除语义

DELETE document → 级联删 chunks/embeddings；`data/uploads/` 文件随删（引用计数为 0 时）。Wiki 已引用该文档的页面不受影响（wiki_sources 存独立副本）。

## 限制与配额

- 单文件 ≤ 50MB；URL ≤ 20MB
- 并发摄取任务数上限（默认 2），排队
- 嵌入失败块单独标记 `embed_failed`，不阻塞整文档 ready（检索时降级为纯 FTS 命中）
