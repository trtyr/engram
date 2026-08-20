/// <reference types="vitest" />
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': import.meta.dirname + '/src',
    },
  },
  // 开发时后端在 :8080（compose 或 cargo run）
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
      '/auth': 'http://localhost:8080',
      '/jobs': 'http://localhost:8080',
      '/memory': 'http://localhost:8080',
      '/knowledge': 'http://localhost:8080',
      '/wiki': 'http://localhost:8080',
      '/codegraph': 'http://localhost:8080',
      '/settings': 'http://localhost:8080',
      '/llm': 'http://localhost:8080',
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    // e2e 归 playwright 跑，vitest 只收单元/组件测试
    exclude: ['e2e/**', 'node_modules/**', 'test-results/**', 'playwright.config.ts'],
  },
})
