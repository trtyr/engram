# wiki-theory-integration

把「LLM Wiki 理论层」补进 `docs/wiki/` 模块文档，并对照理论 + 评论区实战教训输出当前实现的 gap 清单。

## Scope

- **In**：`docs/wiki/` 下新增理论文档与 gap 分析文档；微调 `overview.md` / `README.md` 索引。纯文档，不碰 `crates/` 代码。
- **Out**：gap 里点出的实现类改进（并发去重硬机制、pin 存活、Read Sources Only 等）——记录为开放问题，是否转实现计划由用户另行决定。

## Authority

- 理论来源（用户提供）：`~/Documents/Knowledge Base/01 技术类/05 大模型与 AI/Agent/Wiki 设计/` 下 4 篇：
  - `LLM Wiki 理论说明.md`（Karpathy 原文翻译）
  - `TencentDB-Agent-Memory 实践案例.md`（本项目前身/同源）
  - `llm_wiki 实践案例.md`（nashsu 分支）
  - `LLM Wiki - 评论区实战反馈.md`（实战教训）
- 实现事实：以 `crates/wiki-engine/` 源码 + `docs/wiki/` 已写文档为准。

## File Map

| 文件 | 角色 |
|---|---|
| `roadmap.md` | 任务状态（Done/In Progress/Next/Deferred） |
| `topics/theory-integration.md` | 理论整合方案（往 `docs/wiki/` 写什么） |
| `topics/gap-analysis.md` | gap 分析初步发现（核心） |
| `open-questions.md` | 未决问题 |
