# Implementation Status — agent-memory-platform

> 仅 `In Progress` 阶段的操作交接。权威路线见 [roadmap.md](roadmap.md)。

## Current Phase

（Phase 1 已完成验证，待 compose 终验后关闭；下一步 Phase 2 记忆域）

## Last Landed

- 2026-10 Phase 1 核心底座：全域 schema（16 表）+ jobs + llm + 鉴权 + OpenAPI 快照。
  16/16 测试绿 + 真实网关连通证据（chat 1173ms / embed 480ms，记账落库，密钥密文）。
  证据 [evidence/README.md](evidence/README.md)
- 2026-10 `b79e67f` Phase 0 项目地基（详见 roadmap Done）

## Active TODO

- [ ] compose Phase 1 代码重建终验（后台构建中）→ 通过即提交并关 phase-1 任务
- Phase 2 开工清单：distill crate（提示词模板/五阶段 job）→ memory 域 API（L0 写入/
  atoms/scenarios/persona/检索/context 包）→ jieba 预分词 tsv → e2e 脚本（真 LLM）

## Blocked By

（无）

## Last Verified

- 2026-10（Phase 1 出口）：workspace 16/16 测试绿（fmt/clippy 0 警告）；
  真实网关验证脚本 `scripts/verify-real-provider.sh` 全过（连通/记账/密文三证据）；
  auth 401/403 矩阵 + OpenAPI 13 端点快照绿
- 2026-10（Phase 0 出口）：compose 栈 `/health`→`{"status":"ok"}`、`/ready`→`{"status":"ready","migration_version":1}`、
  `/openapi.json` 正常、404 统一错误体。本地端口 19180（8080/18080 被其他项目占用）
- deploy/.env 为本地验证临时文件（gitignored），含测试密钥，勿提交
- 真网关 newapi.trtyr.top 的 key 在 ~/.pi/agent/auth.json（newapi 条目），不在仓库中

## Phase 0 交付物清单（对照 phases/phase-0-foundation.md）

- [x] git init + .gitignore + .dockerignore
- [x] server/ workspace：9 crates（api 完整骨架，storage 真实现，其余 7 个空壳立边界）
- [x] web/：Vite + React 19 + TS + Tailwind v4 + shadcn/ui（radix preset）+ 路由壳（七域导航）
- [x] deploy/：多阶段 Dockerfile（cargo-chef 分层缓存）+ compose（pgvector + app + healthcheck）+ .env.example
- [x] CI：backend（fmt/clippy/test）+ web（lint/tsc/build）+ docker build 三 job
- [x] AGENTS.md + README.md
- [ ] compose 起栈端到端验证（进行中）
