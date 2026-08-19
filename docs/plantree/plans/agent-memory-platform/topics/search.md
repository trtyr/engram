# Topic — 混合检索

`search` crate 单入口服务，全部域检索复用同一套融合与预算逻辑。

## 流水线

```text
query → 应用层分词（jieba-rs，中文友好）+ embedding（按路由）
      ├─ FTS：tsvector @@ 查询（simple 配置，应用层已预分词）
      ├─ ANN：pgvector 余弦距离（HNSW）
      └─ RRF 融合（k=60）→ 去重 → 预算裁剪 → 带来源返回
```

## 中文方案（Q1，倾向方案 A）

- **A 应用层预分词**：写入与查询两侧都用 jieba-rs 切词后拼 tsvector（`simple` 配置）。零 PG 扩展依赖，镜像不用编译 zhparser；分词质量可接受。
- B zhparser 扩展：体验最好但要自编译进 pgvector 镜像，维护成本高
- C pg_trgm 兜底：作为 A 的补充（子串匹配场景）
- 无论 FTS 质量如何，向量检索兜底语义召回

## 预算控制（防上下文爆炸，TDAM 验证的模式）

`budget: {max_items, max_chars}`，默认 items=20 / chars=8000。各 scope 可分别设限：
`POST /search`（全局）接受 `scopes: [memory, knowledge, wiki]` 加权分配。

## 结果形态

统一 `SearchHit { scope, ref_id, title, snippet, score, source_refs }`——AI 拿到即可引用，不需要二次猜来源。

## 索引

- HNSW：`vector_cosine_ops`，m=16 ef_construction=64（入库量级小，够用；量大后再调）
- GIN：tsvector
- 重建：全量 reindex 作为管理端点（settings，管理员），不自动
