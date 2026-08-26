# 覆盖矩阵与脚本架构

## 脚本架构

```text
scripts/e2e/
├── _lib/
│   ├── env.py      # 编排：本机 PG 建独立 E2E 库 → cargo build → 起服务 → 健康等待 → 清理
│   ├── client.py   # API client：登录/admin token/api-key/请求封装/轮询等待 job
│   └── check.py    # 断言辅助：小而吵的 assert，失败打印完整上下文
├── run_all.py      # 顺序跑全部 test_*.py，汇总结果（也可单独跑）
└── test_*.py       # 每个测试项一个，独立入口：python test_xxx.py → exit 0/1
```

约定：纯标准库 + `requests`（无 pytest、无框架）；每脚本自管环境（复用 env.py 的幂等启动，`--keep` 保留现场排障）；失败时打印服务日志尾部。

## 环境（无 Docker —— 部署链路不属于本计划）

- **PG**：本机 Homebrew PostgreSQL（127.0.0.1:5432，已确认 pgvector 0.8.6 可用）。
  E2E 用独立库 `agent_memory_e2e`（每次跑前 DROP+CREATE，与用户的 `agent_memory` 库完全隔离）；
  迁移由服务启动自动执行（0001 自带 `CREATE EXTENSION IF NOT EXISTS vector`）。
- **服务**：脚本 `cargo build -p agent-memory-api` 后拉起二进制（随机空闲端口），
  `AGENT_MEMORY_DATABASE_URL` 指向 E2E 库；跑完杀进程。
- **codegraph**：本机有 node + CLI 才跑，否则 skip（不打 fail）。

## 覆盖矩阵（域 × 关键旅程）

| # | 脚本 | 旅程 | 核心断言 |
|---|---|---|---|
| 1 | test_health_auth.py | 健康检查 + 登录成功/错密码 401 | /health /ready、token 形态、错误体契约 |
| 2 | test_apikeys_scopes.py | 签发 api-key → scope 隔离（403）→ 吊销后失效 | scope 语义、revoked 后 401 |
| 3 | test_memory_distill.py | 写会话(矛盾两轮) → 蒸馏链 extract→arbitrate→organize→persona | L1 原子存在、矛盾 supersede、L2 场景、L3 画像、evidence 链 |
| 4 | test_memory_read.py | search 分层命中 + context_pack 相关性 + atom 治理 + persona 回滚 + erase_session | 检索命中、context 含相关原子、回滚版本化 |
| 5 | test_knowledge_upload.py | 上传文本 → parse→chunk→embed → ready → chunks 采样 → search → 删除 | 状态机流转、chunk 数、FTS/向量命中、删除级联 |
| 6 | test_wiki_ingest.py | 设 purpose → ingest 文本 → analyze→generate → 页面/互链/index | 页面存在、sources 溯源、purpose 注入、review 项落库 |
| 7 | test_wiki_governance.py | origin=human 保护（LLM 只提案不覆盖）→ lint → insights → archive_query → 级联删除 | human 页不被覆盖、CascadeReport 正确 |
| 8 | test_unified_search.py | 种子三域数据 → POST /search | 三域标签齐全、score 归一化 |
| 9 | test_jobs.py | 任务列表/事件流 → 造永久失败任务 → dead → revive | 事件时间线、dead→pending 复活 |
| 10 | test_llm_settings.py | provider CRUD + routing 读写 + usage 记录出现 | 回退链生效、用量记账行存在 |
| 11 | test_search_recall.py | 长查询（多 token）召回不再零命中（R2 行为验证） | OR 兜底生效（需要无 embedding 场景） |
| 12 | test_codegraph.py | 注册项目→索引→同步→查询（**可选**：本机有 node+CLI 才跑） | 注册/查询返回结构 |

## 现有资产（不复做，作为参照）

- `scripts/verify-memory-e2e.sh`：memory 域 shell 版（3 号脚本的蓝本）
- `scripts/verify-ai-loop.sh`：AI 闭环 shell 版
- crates 内 testcontainers 集成测试：DB 层行为已覆盖，E2E 聚焦 HTTP 面与跨域编排
