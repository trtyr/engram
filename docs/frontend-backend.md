# 前后端接合

> 全栈项目专有文档：前端怎么够到后端。2026-09-01 实查（vite.config.ts / web_assets.rs / auth.rs / api.ts）。

## 三种形态，一条链路

| 形态 | 前端 | 后端 | 说明 |
|---|---|---|---|
| 开发 | Vite :5173 | cargo run :8080 | Vite 代理九前缀（/api /auth /jobs /memory /knowledge /wiki /codegraph /settings /llm）→ VITE_PROXY_TARGET |
| 本地整栈 | 构建 dist 后 rust-embed（debug 直读磁盘） | 同端口 | 改前端无需重编 server |
| 生产 | web/dist 编进二进制 | 单端口同源 | 零 CORS（tower-http cors feature 在 Cargo.toml 但未挂中间件——不需要） |

## 认证流

```text
Login 页 ──POST /auth/login（管理员密码）──▶ ams_ token
       └─▶ localStorage am_token ──▶ api.ts 每次请求附 Bearer
挂载探活：GET /jobs?limit=1（复用碰撞路径做探针）
  仅 401（凭证失效）→ 回登录页；5xx（服务抖动/部署窗口）保持会话不误杀
中途失效：任何 401 ─▶ clearToken + engram-auth-expired 事件 ─▶ App 回 /login
Agent 侧：settings 页签发 amk_ API Key（七 scope：memory/knowledge/wiki/codegraph/llm/erase/cron）
```

## /jobs 路由冲突（2026-08-30 定稿）

前端路由 /jobs 与 `GET /jobs` API 同路径。硬刷新/书签直达时请求先到后端——
认证层按 Accept 分流：`text/html*`（浏览器导航特征）→ rust-embed 回 index.html（SPA 挂载后走壳内路由）；
其余（fetch 默认 `*/*`、curl、API 客户端）→ 照常 Bearer 认证返回 JSON。
这是唯一发生碰撞的路径（其余前端路由的后端 API 均在子路径）。

## 契约与版本

- 版本单源：`web/vite.config.ts` 构建时读 `server/Cargo.toml` workspace version 注入 `__APP_VERSION__`
  （读不到时回退 "dev"——Docker 构建上下文防御，见 deploy/Dockerfile 的对应 COPY）。
- 类型双轨：手写 `lib/api.ts`（页面消费形状）+ 生成 `lib/api-schema.ts`（openapi-typescript，
  CI 零漂移门禁）。**改 utoipa 注解的 struct 必须同 commit 重生成**，否则下一个 push 必红。
- 202 响应带体：api.ts 对 202 做 JSON 解析（204 才是 undefined）——`POST /memory/distill` 返回
  Job[] 依赖此行为。
