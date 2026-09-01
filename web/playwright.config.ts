import { defineConfig } from '@playwright/test'

// e2e：只允许打一次性栈。E2E_BASE 必须显式指定（CI 传 compose 栈地址）。
// 三次清空/误删事故的教训：本地手跑静默打生产库（旧默认 :19180）烧过场景、
// 原子、实体。没有 E2E_BASE 就拒跑——要本地跑请用 scripts/e2e-local.sh 一次性栈。
const BASE = process.env.E2E_BASE
if (!BASE) {
  throw new Error(
    '缺 E2E_BASE：journey 只许打一次性栈，禁止默认指向生产库。' +
      '本地请跑 scripts/e2e-local.sh（起独立 DB 的一次性栈并自动注入 E2E_BASE）。',
  )
}

export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: BASE,
    launchOptions: {
      args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
    },
  },
})
