# 架构（全栈集成视角）

> 模块内部细图各自有档案：[server/docs/architecture.md](../server/docs/architecture.md)、
> [web/docs/architecture.md](../web/docs/architecture.md)。本文写两端的接缝。

## 系统总图

```text
┌────────────────────────── 浏览器 ──────────────────────────┐
│  Engram SPA（web/，React 19）                              │
│  七域页 + 壳（侧栏/命令面板/主题引擎）                       │
└───────────────┬────────────────────────────────────────────┘
                │ 同源 fetch（Bearer ams_/amk_）
┌───────────────▼──────────────── agent-memory-server ───────┐
│ axum Router（55 路径，9 域）                                │
│ ├─ 认证层 bearer_auth（/jobs 对 text/html 分流回 SPA）       │
│ ├─ rust-embed：web/dist 静态托管（生产单二进制）             │
│ └─ 域 crate：distill / wiki-engine / cg-bridge / search     │
│    横切：jobs 队列、llm 网关、storage 仓储、parsing 解析     │
└───────┬──────────────────────────────┬─────────────────────┘
        │ sqlx                          │ reqwest（任务化异步）
┌───────▼──────────┐          ┌────────▼──────────┐
│ PostgreSQL+pgvector│         │ 外部 LLM API       │
│ 19 业务表/14 迁移   │         │ （OpenAI 兼容系）   │
└───────────────────┘          └───────────────────┘
                另：cg-bridge 调用外部 codegraph CLI（Node+git）
```

## 前后端接缝（关键决策）

| 接缝 | 方案 | 出处 |
|---|---|---|
| 开发联调 | Vite 代理九前缀（/api /auth /jobs /memory …）→ :8080 | web/vite.config.ts |
| 生产托管 | rust-embed 把 web/dist 编进 server 二进制，同源零 CORS | server/crates/api/src/web_assets.rs |
| 契约 | utoipa OpenAPI → openapi-typescript 生成 api-schema.ts，CI 零漂移门禁 | ci.yml api-types job |
| 认证 | POST /auth/login → ams_ 会话（前端 localStorage）；API Key amk_ 带 scope | server auth.rs |
| 长操作 | 全部任务化：HTTP 触发入队 → 前端轮询 /jobs + 页面状态 | jobs crate |
| /jobs 路由冲突 | 前端路由与 API 同路径；认证层 Accept 分流（text/html→SPA） | auth.rs（2026-08-30） |
| 版本单一来源 | vite define 从 server/Cargo.toml 注入 __APP_VERSION__ | web/vite.config.ts |

## CI/CD 形状

- `ci.yml` 四 job：backend（fmt/clippy/test，pgvector service）/ web（lint/tsc/test/build）/
  api-types（schema 零漂移）/ docker（多阶段构建验证）
- `e2e.yml`：compose 全栈 + Playwright journey（无 provider 部分旅程）
- 详见 [conventions.md](conventions.md) 与 [server/docs/conventions.md](../server/docs/conventions.md)
