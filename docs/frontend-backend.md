# 前后端连接

> web/ 如何够到 server/：开发代理、生产同源、类型契约、鉴权流。这是 fullstack 归档的专属文档（server 侧旧归档因 backend-only 跳过了它）。

## 开发：Vite 代理（无 CORS）

`web/vite.config.ts` 把 9 个前缀代理到后端（默认 `http://localhost:8080`，可用 `VITE_PROXY_TARGET` 覆盖）：

```text
/api /auth /jobs /memory /knowledge /wiki /codegraph /settings /llm
```

前端代码里 `lib/api.ts` 的 `BASE = ''`（同源相对路径）——开发请求打 Vite dev server，由代理转发到后端；因此**开发与生产都不需要 CORS**（服务端 Cargo.toml 虽带 tower-http `cors` feature，路由装配未挂 CORS 中间件，也没必要）。

## 生产：单端口同源（rust-embed）

- `deploy/Dockerfile` 三阶段：node 构建前端 → cargo-chef 构建后端（rust-embed 编译期内嵌 `web/dist`）→ debian trixie 运行时。
- `server/crates/api/src/web_assets.rs` + `routes/mod.rs` 的 `fallback_service`：**API 路由未命中的路径兜底到 SPA 静态资源**。浏览器只看到一个 `:8080` 源。

## 类型契约：OpenAPI → TypeScript

```text
server 代码（#[utoipa::path] 标注）
  │ cargo run -p agent-memory-api --bin openapi-dump   （不起服务，代码提取）
  ▼
openapi.json（55 paths）
  │ npx openapi-typescript（或 web 的 `pnpm run gen:api`，读 OPENAPI_URL，默认 :8090）
  ▼
web/src/lib/api-schema.ts（2941 行生成类型，提交进库）
```

CI `api-types` job 每次重新生成并 `git diff` 检查漂移——**后端 API 变更必须同步再生成并提交，否则 CI 红**。2026-08-28 验证：当前 HEAD 零漂移。

注意：前端还有一份手写的域类型（`lib/api.ts` 里的 `Session`/`Atom`/`WikiPage` 等 interface）——`api-schema.ts` 是生成的权威，手写类型是消费侧便利层（见 `web/docs/api-alignment-audit.md` 的对接审计）。

## 鉴权流（前端视角）

```text
Login 页（features/Login.tsx）
  │ POST /auth/login {password}          （admin 密码 = AGENT_MEMORY_ADMIN_PASSWORD）
  ▼ ams_ token（明文只此一次）
localStorage['am_token']
  │ lib/api.ts 每请求注入 Authorization: Bearer <token>
  ▼
401 响应 → clearToken() → App.tsx 探活失败 → 跳 /login
```

- `App.tsx` 挂载时用 `GET /jobs?limit=1` 探活判断登录态。
- API key（`amk_`）也可用于开发调试（api.ts 注释明示），scope 限定四域。
- e2e（Playwright 与 Python 套件）同样走 `/auth/login` 拿 admin token，密码经 `E2E_ADMIN_PW` 注入、不写死。

## 前端页面对端点的消费面

七域一页一域（features/ ↔ 后端路由组）：Memory↔/memory/*+蒸馏触发、Knowledge↔/knowledge/*（upload 走 multipart）、Wiki↔/wiki/*（含 sigma 图谱渲染 /wiki/graph）、CodeGraph↔/codegraph/*、Jobs↔/jobs/*、Settings↔/settings/*+/llm/usage、Dashboard 聚合。跨域检索 `POST /search` 在多页复用。完整端点表见 [api.md](api.md)。
