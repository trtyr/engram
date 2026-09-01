# Wiki 模块文档

这是 `agent-memory` 后端 **Wiki 功能模块** 的深度文档，覆盖实现细节、数据模型、运行链路与操作面。范围：backend-only，对应 `crates/wiki-engine` + `crates/core/src/wiki.rs`（门面）+ `crates/api/src/routes/wiki_api.rs`（HTTP 层）。

## 一句话定位

Wiki 是四类长期记忆资产之一，采用 **Karpathy LLM-wiki 模式**：原料（source）不可变，LLM 增量维护互链页面，人负责纠偏（review / purpose / 人工编辑保护）。

## 文档索引

| 文档 | 内容 | 何时读 |
|---|---|---|
| [overview.md](overview.md) | Wiki 是什么、设计哲学、页面类型全景、整体数据流 | 想 30 秒建立心智模型时 |
| [ingest.md](ingest.md) | 两步 ingest 全链路（analyze → generate）、幂等、origin 保护、系统页维护 | 想知道「一篇文档怎么变成 wiki 页面」时 |
| [data-model.md](data-model.md) | 5 张表 + settings 里 purpose 的字段/枚举/frontmatter 语义 | 想查数据库结构时 |
| [graph.md](graph.md) | 链接图、4 信号相关性模型、Louvain 社区、图洞察 | 想理解页面之间如何关联时 |
| [operations.md](operations.md) | lint 规则、review 系统、purpose、级联删除、检索 | 想做人工运维/纠偏时 |
| [api.md](api.md) | 全部 HTTP endpoint、鉴权、请求/响应/错误契约 | 想写客户端/调接口时 |
| [theory.md](theory.md) | 设计理论与来源：Karpathy 三层架构/三操作/index+log + 理论→实现映射 | 想懂「为什么这么设计」时 |
| [gap-analysis.md](gap-analysis.md) | 对照理论最佳实践的差距清单（已补/未补） | 想做改进规划时 |

## 相关文档

- 顶层 [architecture.md](../architecture.md)：wiki-engine 在 workspace 中的位置与依赖方向
- 顶层 [api.md](../api.md)：wiki 与其他域的接口总览
- 顶层 [data-model.md](../data-model.md)：全部 22 张业务表的全局视角
