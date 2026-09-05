# 运行与部署

> 2026-08-30 实跑验证。

## 前置

- Node ≥22、pnpm 11.20.0（corepack 或自带）
- `web/.npmrc`：`legacy-peer-deps=true`

## 开发

```bash
cd web
pnpm install --frozen-lockfile
pnpm dev            # Vite :5173，/api /auth /jobs /memory … 代理到 VITE_PROXY_TARGET（默认 :8080）
```

代理表在 `vite.config.ts`（九个前缀）；后端用 cargo run 起（见 server/docs/run-and-deploy.md）。

## 质量门禁（当日全绿）

```bash
pnpm run lint       # oxlint；⚠️ 必须带 run（裸 pnpm lint 误报 eslint 缺失）
pnpm exec tsc --noEmit
pnpm test           # vitest 53/53（jsdom）
pnpm run build      # tsc -b && vite build → dist/
```

## e2e（Playwright journey + wiki-ia）

```bash
# 需要运行中的栈（默认 E2E_BASE=http://127.0.0.1:19180）
E2E_ADMIN_PW=<管理员密码> pnpm exec playwright test
```

两个 spec（testDir e2e/）：
- **journey.spec.ts**：登录→上传文档→ready→写会话→（有 provider 才走）蒸馏断言→wiki→
  （有 provider 才走）摄取断言→codegraph 注册/索引/查询。无 LLM provider 的栈跑部分旅程（探测 /settings/llm/providers）。
- **wiki-ia.spec.ts**（2026-09-03 新增）：Wiki 目录树——折叠持久化 / URL 写回 / ?page= 深链断言。

配置：`playwright.config.ts`（E2E_BASE 必填拒跑打生产库；SwiftShader WebGL，600s 超时）。

## 类型再生成

```bash
pnpm run gen:api    # OPENAPI_URL 指向运行中后端的 openapi.json → src/lib/api-schema.ts
```

## 构建产物去向

- 本地/CI：`web/dist`（gitignore）。
- 生产：后端 rust-embed 把 `web/dist` 编进二进制同源托管——改前端后需重建 dist 并重编 server
  （debug 模式 rust-embed 直读磁盘，无需重编）。
- CI 后端 job 需要 `mkdir -p ../web/dist` 占位（rust-embed 编译期检查目录存在）。

## 设计验证脚本（仓库内，非产品代码）

`web/e2e-design-shots.mjs`（双主题截图）、`web/e2e-design-metrics.mjs`（计算样式度量）——
设计审计用，可直接 node 运行。
