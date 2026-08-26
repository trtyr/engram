import { defineConfig } from '@playwright/test'

// Phase 6/7 e2e：对运行中的栈（默认 127.0.0.1:19180）跑全旅程
export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: process.env.E2E_BASE ?? 'http://127.0.0.1:19180',
    launchOptions: {
      args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
    },
  },
})
