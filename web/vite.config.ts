/// <reference types="vitest" />
import { readFileSync } from 'node:fs'
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// 版本单一来源：server/Cargo.toml [workspace.package] version（构建期注入，前端不再手写）。
// 防御：文件缺席（如裁剪过的构建上下文）时退回 'dev'，不让版本注入破坏构建。
const VERSION_FALLBACK = 'dev'
let APP_VERSION = VERSION_FALLBACK
try {
  const serverToml = readFileSync(new URL('../server/Cargo.toml', import.meta.url), 'utf8')
  APP_VERSION = serverToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? VERSION_FALLBACK
} catch {
  /* server/Cargo.toml 不可达——用 fallback */
}

// https://vite.dev/config/
export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(APP_VERSION),
  },
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': import.meta.dirname + '/src',
    },
  },
  // 开发时后端在 :8080（compose 或 cargo run）
  server: {
    proxy: Object.fromEntries(
      ['/api', '/auth', '/jobs', '/memory', '/mcp', '/wiki', '/codegraph', '/settings', '/llm'].map(
        (p) => [p, process.env.VITE_PROXY_TARGET ?? 'http://localhost:8080'],
      ),
    ),
  },
  build: {
    // mermaid 主入口（~662kB）是发布产物固有体积，仅在渲染 mermaid 图时按需加载
    // （各图表类型已自动分 chunk；域页已路由级 lazy——初始 bundle ~280kB）。故放宽阈值。
    chunkSizeWarningLimit: 800,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    // e2e 归 playwright 跑，vitest 只收单元/组件测试
    exclude: ['e2e/**', 'node_modules/**', 'test-results/**', 'playwright.config.ts'],
  },
})
