# 当前状态（2026-09-08 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

**七域 MCP 渐进式发现 + Wiki 多库 + 全链路治理成型**：`/mcp` **7 个入口工具**
（memory / projects / skills / wiki / todos / codegraph 六域 + 跨域 **search_all**）共 **63 个域内
操作**按 action 分发——L0 描述内嵌操作目录（含动态资产清单织入）+ L1 action="help" 参数手册 +
L2 错误自愈，按 key scope 分权，管理台两级开关（整域隐身 / 单操作停用）。**Wiki 多库**：库 =
一级命名空间（页面/双链/原料/文档/审查/purpose 全按库隔离），织入产物落同库，同名 slug 跨库合法。
持久化全面收口 `engram-storage::repo`（core/api/mcp src 层零 sqlx，11 crates，MCP 与 HTTP 平级
双适配器）。cargo **243** 测试 / vitest **59**（11 文件）/ **122 路径** / **37 迁移** / **34 业务表**。

## 2026-09 当期能力（近期大项）

- **Wiki 真多库**：建库/删库（force 级联）、`?lib=` 全端点、MCP `library` 参数 + `libraries`
  操作、purpose 每库、织入库传播、同名 slug 跨库合法（双库实机验收 12/12 + 黑盒 27/27）；
- **记忆治理**：会话批量恢复/擦除、单会话恢复按钮、遗忘三态（void/erase/restore）、
  画像退休语义（空版本不返回）、list_atoms 默认 active、凭据不落原子（蒸馏 v5）；
- **上下文瘦身**：写操作不回显正文（content_omitted + content_chars）、wiki 检索片段化、
  codegraph explore 符号大纲化、search_all 跨域一次查；
- **版本与原料**：wiki 版本快照/回滚/删页重建、织入原料清理通道；
- **体验**：JetBrains Mono + 霞鹭文楷字体分工、全站应用内确认弹窗、待办与 Wiki 的批量操作。

## 当日验证矩阵（活体）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt / clippy --workspace --all-targets | 0 / 0 警告 |
| server | cargo test --workspace | 243 passed / 0 failed |
| web | pnpm tsc -b / vitest run / oxlint src / build | 0 / 59 passed（11 文件）/ 0 新增警告 / 成功 |
| 事实 | OpenAPI 快照（openapi_snapshot 活体） | **122 路径** / 37 迁移 / 34 业务表 |
| 实机 | 双库全链验收 + 多库黑盒专项 | 12/12 + 27/27（scripts/zztest_m10.py） |

## 分栈细节

- [server/docs/current-state.md](../server/docs/current-state.md)（各域形态 + 已知边界）
- [web/docs/current-state.md](../web/docs/current-state.md)
- MCP 工具面全表：[server/docs/mcp.md](mcp.md)
