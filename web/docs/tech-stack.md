# 技术栈

> 2026-08-30 从 pnpm-lock.yaml 与 package.json 实查。

## 基础

| 项 | 版本 | 说明 |
|---|---|---|
| react / react-dom | 19.2.8 / 19.2.5 | UI 运行时 |
| typescript | 6.0.3 | `tsc -b` 进 build |
| vite | 8.2.2 | 构建（@tailwindcss/vite 4.3.3 插件） |
| tailwindcss | 4.3.3 | CSS 引擎（CSS-first 配置，token 全在 index.css） |
| pnpm | 11.20.0 | 包管理（packageManager 钉版；.npmrc legacy-peer-deps） |
| Node | 22（CI/Docker）；本机更高 | — |

## 关键依赖

| 包 | 版本 | 用途 |
|---|---|---|
| react-router-dom | 7.18.2 | 路由 |
| @tanstack/react-query | 5.102.3 | 服务端状态（部分页面） |
| react-markdown | （lockfile） | Wiki 正文渲染 |
| mermaid | 11.17.2 | 图表（懒加载，662kB 独立 chunk） |
| sigma + graphology | 3.0.3 | Wiki 图谱（WebGL） |
| cytoscape | （懒加载 435kB） | CodeGraph 结构查询可视化 |
| lucide-react | 1.34.0 | 图标（唯一图标体系，无 emoji） |
| radix-ui | 1.6.7 | 无障碍基元（部分组件） |
| @fontsource-variable/geist(-mono) | 5.3.0 | 展示与等宽字体 |

## 工具链

| 工具 | 版本 | 命令 |
|---|---|---|
| oxlint | 1.80.0 | `pnpm run lint`（⚠️ 裸 `pnpm lint` 会误报 eslint 缺失——必须 `run`） |
| vitest | 4.1.11（jsdom） | `pnpm test`（35 用例） |
| @playwright/test | 1.62.1 | `pnpm exec playwright test`（journey，SwiftShader WebGL） |
| openapi-typescript | 7.13.0 | `pnpm run gen:api` → api-schema.ts |

## 构建产物（2026-08-30 实测）

- 初始 JS gzip ~93kB + CSS 8.5kB（预算 <350kB，富余充足）
- 懒加载大块：mermaid 662kB / Wiki 路由 295kB / cytoscape 435kB——全部在 lazy 边界后
- vite define：`__APP_VERSION__` ← server/Cargo.toml（跨栈版本单一来源）
