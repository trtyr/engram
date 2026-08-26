# Test & Release Gates — 质量门与发布

> 成熟产品要求：每阶段交付即生产质量，不走「demo 后补测试」。

## 后端（Rust）

| 门 | 工具 | 要求 |
|---|---|---|
| 格式/静态 | `cargo fmt --check` + `cargo clippy -- -D warnings` | 零警告 |
| 单元测试 | `cargo test` | 领域逻辑（蒸馏编排、RRF、wikilink 解析、矛盾消解、job 状态机）全覆盖核心路径+边界+失败路径 |
| 集成测试 | testcontainers（真 PG + pgvector） | 仓储层、迁移、检索（含中文）、事务 |
| API 契约 | utoipa 生成 OpenAPI + 快照测试 | 端点变更必须显式更新快照 |
| LLM 依赖 | mock provider（trait 注入） | 蒸馏管道测试不调真模型 |

## 前端（React/Vite）

| 门 | 工具 | 要求 |
|---|---|---|
| 类型/静态 | `tsc --noEmit` + eslint | 零错误 |
| 单元 | vitest + testing-library | 关键组件与 hooks |
| E2E | playwright（对 docker-compose 起的真栈） | 关键用户旅程：上传文档→ready、写入会话→蒸馏完成→画像更新、Wiki ingest→页面出现 |

## API 类型同步

前端类型从 OpenAPI 生成（openapi-typescript），禁止手写重复类型。

## CI（GitHub Actions）

1. fmt + clippy + cargo test（含 testcontainers）
2. tsc + eslint + vitest
3. 前端 build + 后端 build + OpenAPI 快照
4. docker build 验证（多阶段构建可出镜像）
5. e2e（compose 栈上跑 playwright）

## 发布交付

- `deploy/Dockerfile`：多阶段（node:22 构建前端 → cargo 构建后端(嵌入静态资源) → gcr.io/distroless 或 debian-slim 运行时）
- `deploy/docker-compose.yml`：app + `pgvector/pgvector:pg17`，健康检查，卷声明，自动迁移
- `.env.example` 完整变量样例
- `README.md`：一键启动 + API 快速上手（给 AI 看的接入说明——平台即工具，文档就是工具说明书）

## 验收基准（每阶段通用）

- 该阶段功能从 docker-compose 起栈可用
- 上述质量门全绿
- 文档（README/API 文档/UI 内文案）与实现同步
