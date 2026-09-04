# Wiki 设计理论

本文是 Wiki 模块的「为什么」层：理论来源、设计意图，以及它们如何落到当前实现。实现细节见 [ingest.md](ingest.md)、[graph.md](graph.md)、[operations.md](operations.md)。

## 理论谱系

```text
Karpathy《llm-wiki.md》(2026)
   ├── TencentDB-Agent-Memory  —— 团队级记忆中枢（本项目前身/同源）
   ├── nashsu/llm_wiki          —— 个人级桌面应用
   └── Engram（本项目）    —— 团队级思路的 Rust 重写，定位单用户平台
```

理论来源（用户知识库 `~/Documents/Knowledge Base/01 技术类/05 大模型与 AI/Agent/Wiki 设计/` 下 4 篇 + 原文）：

- `LLM Wiki 理论说明.md` — Karpathy 原文翻译
- `TencentDB-Agent-Memory 实践案例.md`
- `llm_wiki 实践案例.md`
- `LLM Wiki - 评论区实战反馈.md`
- [Karpathy 原文 gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f)

## 核心思想

大多数「LLM + 文档」是 RAG：查询时检索片段、现场拼答案，**每次提问都从零重新发现知识，没有任何积累**。

Karpathy 的模式相反：LLM **增量构建并维护一个持久 wiki**——一个结构化、互相链接的 markdown 集合，横亘在你和原始来源之间。知识**编译一次，然后持续保鲜**：

- 交叉引用已经在那里了
- 矛盾已经被标注了
- 综述已经反映了你读过的所有东西

> wiki 是一个持久的、会复利增长的产物。Obsidian 是 IDE，LLM 是程序员，wiki 是代码库。

**分工**：LLM 做全部苦差事（总结、交叉引用、归档、记账）；人负责策展来源、引导分析、提出正确的问题。维护成本几乎为零，所以 wiki 不会被放弃。

## 三层架构

| 层 | Karpathy 定义 | 本项目落点 |
|---|---|---|
| **原始来源（Raw sources）** | 精心策展的源文档，**不可变**，LLM 只读不写，是真相来源 | `wiki_sources` 表 + 落盘 `{data_dir}/wiki-sources/{id}.md` + `sha256` 去重 |
| **wiki** | LLM 生成的 markdown 目录，**LLM 完全拥有**：建页、更新、维护交叉引用 | `wiki_pages`（`origin='llm'`）+ `wiki_generate` job |
| **规范（schema）** | 一份文档告诉 LLM wiki 如何组织、有哪些约定 | `prompts.rs` 的 `analysis_system` / `generation_system`（版本化） |

`llm_wiki` 给这套补了一个关键的第四维——**purpose（方向意图）**：

> schema 是结构规则，purpose 是方向意图。

schema 只能保证「页面格式规整」，没法告诉 LLM「这个 wiki 到底想搞明白什么」。purpose 放目标、关键问题、研究范围、不断演进的论点。本项目落点为 `purpose.rs` + `settings[wiki_purpose]`，ingest/query 时注入 LLM，LLM 可建议更新（经人审）。

## 三操作

| 操作 | 意图 | 本项目落点 |
|---|---|---|
| **ingest（收录）** | 读来源 → 写摘要页 → 更新 index → 更新相关实体/概念页 → 追加 log | `enqueue_ingest → wiki_analyze → wiki_generate`（两阶段） |
| **query（查询）** | 搜相关页 → 综合带引用答案；**好答案归档回 wiki** | `search`（FTS+向量）；`archive_query`（落 queries 页） |
| **lint（体检）** | 查矛盾/过时论断/孤儿页/缺交叉引用/数据缺口 | `lint.rs`（5 规则） |

### 为什么拆成两阶段 ingest

Karpathy 原文的 ingest 是一气呵成的。TencentDB 与 llm_wiki **独立收敛**到「分析 + 生成」两段：

1. **分析**：LLM 只产结构化抽取计划（实体/概念/关联/冲突），不写页。
2. **生成**：拿着计划产最终页面。

源码注释讲得直白：**把「抽什么」和「落盘格式」解耦，质量更稳、格式更规整**。拆开后分析阶段能先看到已有页面清单，判断「新建 vs 合并更新」，从根上抑制近重复页。

### 什么值得单独成页：granularity 三问

TencentDB 补的关键细节，判断一个主题要不要单独建页：

1. **独立身份**——能否脱离父上下文被独立理解？
2. **独特关系**——跟别的实体有没有「属于父级」之外的有意义关系？
3. **内容充实**——有没有超过一句话占位的东西可写？

三问全满足才建页。本项目落点在 `prompts.rs` 的「宁缺毋滥：只在文中**反复出现或为核心主题**时列出」约束。

## index.md 与 log.md

Karpathy 说有两个特殊文件帮 LLM（和人）导航：

| 文件 | 面向 | 意图 | 本项目落点 |
|---|---|---|---|
| `index.md` | 内容 | 全内容目录（每页链接 + 一行摘要 + 元数据）；回答查询时先读 index 找页再钻 | `index` 系统页（ingest 后重建） |
| `log.md` | 时间 | 只追加记录（ingest/query/lint），统一前缀可被 `grep` 解析 | `log` 系统页 + `job_events` 全量历史 |

`index` 的有损性（一行摘要浮不出正文深埋的事实）是评论区反复踩的坑，本项目靠 **FTS + 向量检索**绕过 index 导航瓶颈（详见 [operations.md](operations.md)）。

## 理论 → 实现映射表

| 理论概念 | 实现落点 |
|---|---|
| 原始来源不可变 | `wiki_sources` + 落盘 `.md` + `sha256` |
| wiki 层 LLM 全权 | `wiki_pages`（origin=llm）+ `generate_job` |
| 规范 schema | `prompts.rs`（版本化 `PromptId`） |
| purpose 方向意图 | `purpose.rs` + `settings[wiki_purpose]` |
| 两阶段 ingest | `wiki_analyze` → `wiki_generate` |
| 矛盾检测 | analysis 的 `conflicts[]` + `comparison` 页 |
| 4-signal 相关性 | `relevance.rs`（3/4/1.5/1） |
| 异步 Review（预定义动作） | `review.rs` + `wiki_review_items` |
| Graph Insights | `insights.rs`（4 类） |
| 级联删除 | `cascade.rs`（3 路径匹配） |
| lint 体检 | `lint.rs`（5 规则） |
| 好答案归档 | `archive_query` → queries 页 |
| index/log | `index` / `log` / `overview` 系统页 |

## 精神源头

这个模式与 Vannevar Bush 的 **Memex**（1945）一脉相承：私人的、精心策展的知识库，文档之间的关联与文档本身同等珍贵。Bush 没解决「谁来做维护」，LLM 接过了这一棒。

当前实现与理论最佳实践的差距，见 [gap-analysis.md](gap-analysis.md)。
