# Phase 3 — 知识域（Knowledge）

**目标**：文档/URL 摄取到可检索的全管道，生产质量（限流、SSRF 防护、失败降级都真做）。

## 前置

Phase 1（与 Phase 2 可并行/乱序）。

## 交付物

### 摄取管道

- [ ] 上传（multipart，≤50MB）+ URL 提交（SSRF 全套防护：DNS 全记录校验/私网拒/重定向复检/20MB/30s）
- [ ] sha256 文档级去重（重复提交返回已有 id）
- [ ] parsing job：md/txt 直读；pdf（pdf-extract）；docx（docx-rs）；html 正文抽取；原始文件落 `data/uploads/`
- [ ] chunking job：结构感知分块（标题优先，512–1024 token，15% 重叠）
- [ ] embedding job：批量并行（并发上限可配），单块失败标 `embed_failed` 不阻塞 ready
- [ ] 状态机 pending→parsing→chunking→embedding→ready/failed 全程 job_events 进度
- [ ] 并发摄取上限（默认 2）排队

### API

- [ ] documents CRUD（DELETE 级联 chunks/embeddings/文件）+ 分块预览
- [ ] `POST /knowledge/search`：FTS+向量 RRF，结果带文档引用+高亮，预算封顶

## 出口标准

1. e2e（evidence 留档）：上传真实 PDF + md + 一个公网 URL → 全部 ready → 中文查询同时命中三者且引用正确
2. SSRF 测试集：`http://127.0.0.1`、`http://169.254.169.254`、内网域名解析私网、重定向到私网——全部拒绝且错误体不泄漏细节
3. 损坏 PDF / 超限文件 / 坏 URL → failed 状态 + 可读错误 + 不影响其他任务
4. 重复上传同文件 → 秒回已有 id，无二次解析（事件流水验证）
5. mock embedding 单测：分块边界（超长段落/无结构纯文本）、失败降级；Phase 0/1 出口标准依然全绿

## 关联

- 设计：[knowledge-ingest](../topics/knowledge-ingest.md)、[search](../topics/search.md)
- 风险：R6、R7
