# backend-e2e

整个后端的全面 E2E 测试：Python 脚本，**一个脚本 = 一个测试项**，独立可跑、独立退出码。不只 wiki——覆盖鉴权/记忆/知识/wiki/跨域/任务/LLM 设置全部 API 面。

## Scope

- **In**：`scripts/e2e/`（Python）；公共编排库 `_lib/`（起 PG、起服务、登录、API client）；每个测试项一个 `test_*.py`；跑完输出人话摘要。
- **Out**：单元测试/集成测试（已在 crates 内）；前端 e2e（web/ 侧）；性能压测。

## Authority

- API 契约：`server/docs/api.md` + 运行时 `/openapi.json`。
- 编排模式：沿用 `scripts/verify-memory-e2e.sh` 已验证的骨架（docker pgvector → 服务二进制 → 登录 → provider 配置）。

## File Map

| 文件 | 角色 |
|---|---|
| `roadmap.md` | 脚本清单（一脚本一任务，分批）+ 进度 |
| `topics/coverage.md` | 覆盖矩阵 + 脚本架构 |
| `open-questions.md` | 待用户提供的输入 |
