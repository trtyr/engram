# 数据模型（集成索引）

数据库 19 张业务表 + 14 迁移（2026-08-30 运行库实查）。表清单、迁移史、约束陷阱在
[server/docs/data-model.md](../server/docs/data-model.md)；前端消费的类型形状与状态机在
[web/docs/data-model.md](../web/docs/data-model.md)。

## 全栈数据流（一段话版）

会话/文档/wiki 源从控制台或 Agent API 进入 → 全部落 jobs 队列异步处理（蒸馏/分块嵌入/分析生成）
→ 产物分别沉淀为 atoms/chunks/wiki_pages → 检索层（/search、/memory/context、/wiki/search）
把 L1~L3 与向量召回组装回 Agent prompt。LLM 每次调用经网关记 llm_usage。

## 契约要点（改 schema 时三处同步）

1. `server/migrations/`——新迁移只增不改
2. `web/src/lib/api.ts`——手写域类型（页面实际消费）
3. `pnpm run gen:api` 重生成 `web/src/lib/api-schema.ts`（CI 会查漂移）
