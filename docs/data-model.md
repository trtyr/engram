# 数据模型（集成索引）

数据库 24 张业务表 + 25 迁移（2026-09-03 运行库实查；0025 = wiki_pages.folder 目录树）。表清单、迁移史、约束陷阱在
[server/docs/data-model.md](../server/docs/data-model.md)；前端消费形状在
[web/docs/data-model.md](../web/docs/data-model.md)。

分层记忆 + 坐标系：

```text
raw_sessions(L0) → atoms(L1) → scenarios(L2) → persona_aspects(L3)
                      ↕ atom_entities ↕
                  entities（记忆坐标系：人/项目/主题/群组/地点）
```

治理三件套（2026-08-31 起）：`atoms.sensitive`（隐私标记，检索/打包/导出默认排除）、
`atom_revisions`（编辑留痕）、jobs 表的 `cancelled` 态（deep purge 两阶段后悔药）。
