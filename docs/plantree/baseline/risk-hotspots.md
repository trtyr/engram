# Risk Hotspots — 风险热点

| # | 风险 | 影响 | 缓解 |
|---|---|---|---|
| R1 | **中文全文检索质量**：PG FTS 默认无中文分词，zhparser 需要自编译扩展 | 中文检索召回差 | 应用层预分词（jieba-rs）写入 tsvector；向量检索兜底；见 open-questions Q1 |
| R2 | **蒸馏质量依赖提示词**：L0→L3 全链路 LLM 输出不稳 | 记忆污染、画像漂移 | 提示词版本化+产物记录版本；矛盾消解保留历史；UI 可视 diff 供人审；consolidation 定期整理 |
| R3 | **codegraph 上游 breaking change**：CLI 输出格式随版本变 | cg-bridge 解析崩 | pin 版本；CLI `--json` 输出做防御性解析；版本探测；D0005 决策记录 |
| R4 | **长任务可靠性**：ingest/distill 崩溃、重复执行 | 数据重复/任务卡死 | 幂等键、行锁抢任务、running 超时回收、重试上限、job_events 全程可观测 |
| R5 | **LLM 成本失控**：蒸馏+wiki 编译调用频繁 | 账单爆炸 | llm_usage 记账+UI 展示；模型路由（便宜模型干抽取）；每任务预算上限 |
| R6 | **大文档摄取内存**：pdf/长文解析 OOM | 服务崩溃 | 流式解析、分块上限、上传大小限制 |
| R7 | **pgvector HNSW 索引构建慢**：首次大批量嵌入 | 启动慢 | 增量嵌入、后台 reindex |
| R8 | **Docker 镜像内跑 codegraph（Node）**：体积+复杂度 | 镜像 >1GB | codegraph npm 包自带 runtime，官方支持 self-contained 安装；distroless 不行就用 debian-slim；单镜像 vs 拆分的权衡见 D0005 |
| R9 | **API key 安全**：AI 侧 key 泄漏 | 数据暴露 | key 哈希存储、可吊销、scopes 限制；LLM provider key 服务端加密存储 |
