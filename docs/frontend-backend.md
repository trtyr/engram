# 前后端接合

> 全栈项目专有文档：前端怎么够到后端。2026-08-30 实查（vite.config.ts / web_assets.rs / auth.rs / api.ts）。

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
中途失效：任何 401 ─▶ clearToken + engram-auth-expired 事件 ─▶ App 回 /login
Agent 侧：settings 页签发 amk_ API Key（scope 受限）
```

## /jobs 路由冲突（2026-08-30 定稿）

前端路由 /jobs 与 `GET /jobs` API 同路径。硬刷新/书签直达时请求先到后端——
认证层按 Accept 分流：`text/html*`（浏览器导航特征）→ rust-embed 回 index.html（SPA 挂载后走壳内路由）；
其余（fetch 默认 `*/*`、curl、API 客户端）→ 照常 Bearer 认证返回 JSON。
这是唯一发生碰撞的路径（其余前端路由的后端 API 均在子路径）。

## 契约与版本

- OpenAPI：server 导出 → web 生成类型（`pnpm run gen:api`）→ CI 零漂移门禁
- 版本：server/Cargo.toml → vite define `__APP_VERSION__` → 侧栏底部展示，单一来源
- 前端 bundle 预算 350kB gzip（当前 ~102kB，重库全在懒加载边界后）
