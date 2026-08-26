# Roadmap

## Done

| ID | 任务 | 产物 |
|---|---|---|
| T1 | 理论整合：新增 `docs/wiki/theory.md`（三层架构/三操作/index+log + 理论→实现映射） | ✅ 已落 |
| T2 | gap 分析：新增 `docs/wiki/gap-analysis.md`（已补/未补对照清单） | ✅ 已落 |
| T3 | 微调 `overview.md`（延伸阅读段）+ `README.md`（索引两行） | ✅ 已落 |

## In Progress

（空）

## Next

（空——本计划交付后即完成）

## Deferred

gap 分析点出的**实现类改进**（超出本计划「纯文档」范围，见 `open-questions.md`）：

| 缺口 | 来源 | 性质 |
|---|---|---|
| 并发去重硬机制（index 占位符 + 原子条件写入） | 评论区 huachen-wang 教训 1 | 代码改动 |
| pin 存活机制（记录修正意图、重编译后核对） | 评论区 huachen-wang 教训 4 | 代码改动 |
| Read Sources Only 安全阀 | llm_wiki 案例 | 代码改动 |
| 查询侧按类型路由（主题 vs 实体） | 评论区 tonydzi 469 轮 | 代码改动 |
