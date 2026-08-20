# Changelog

本项目遵循 [SemVer](https://semver.org/)。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.1.0] — 2026-10

首个完整版本。八阶段路线（plan-tree）全部交付：

### 新增

- **记忆域（Chat Memory）**：L0 会话 → L1 原子（8 类）→ L2 场景 → L3 画像的分层
  蒸馏管道；矛盾仲裁自动 supersede；画像分面版本化 + 回滚；三层引用链（画像→场景→原子→会话）
  全程可溯源；低置信人审队列。
- **知识域（Knowledge）**：pdf/docx/html/md/URL 摄取（SSRF 防护：DNS pinning、
  私网全拒、逐跳复检、大小/超时限制）；结构感知分块；嵌入（matryoshka 1024 维）；
  失败降级 FTS；sha256 幂等。
- **Wiki 域**：Karpathy 模式两步 ingest（分析→生成）；[[wikilink]] 链接图；
  人工页提案保护（LLM 不覆盖人写内容）；lint（死链/孤儿/重复实体）。
- **CodeGraph**：复用 codegraph CLI（pin 1.5.0）的项目注册/索引/四类查询代理
  （explore/search/callers/callees/impact）。
- **检索**：jieba 中文预分词 + pgvector ANN + RRF 融合；上下文预算控制；
  `/memory/context` 一站式冷启动包。
- **任务系统**：PG-backed 队列（SKIP LOCKED）、指数退避重试、僵尸回收、
  链式入队；每次 LLM 调用完整 I/O 落事件流（可归因可回放）。
- **LLM 出口**：多 provider（OpenAI 兼容）、AES-256-GCM 密钥加密、
  purpose 路由（有序回退链）、用量记账。
- **Web 控制台**：七域 UI（Dashboard/Memory/Knowledge/Wiki/CodeGraph/Jobs/Settings），
  单端口 SPA（rust-embed 嵌入）。
- **交付**：Docker 多阶段单镜像（含 codegraph）、compose（pgvector）、
  备份/恢复脚本、CI（fmt/clippy/test/lint/tsc/vitest/类型漂移/docker build/e2e）、
  [AI 接口文档](docs/AI-INTERFACE.md)（可直接交给 AI 客户端）。

[0.1.0]: https://github.com/trtyr/agent-memory/releases/tag/v0.1.0
