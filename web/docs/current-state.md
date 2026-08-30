# 当前状态（2026-08-30 验证基线）

## 未提交变更

`web/` 侧 24 改 + 6 新增（含 docs/），全部属于 Engram 重设计 + 侧栏四件套批次，
与 `server/crates/api/src/auth.rs` 同批待提交（见 server/docs/current-state.md 的完整清单）。

## 当日验证（终树复跑）

| 命令 | 结果 |
|---|---|
| pnpm exec tsc --noEmit | 0 errors |
| pnpm run lint | exit 0，**0 警告**（set-state-in-effect 已全部清除） |
| pnpm test | **26/26**（5 文件；CommandPalette 新增 5 用例） |
| pnpm run build | exit 0；初始 JS gzip ~93kB + CSS 8.5kB |
| E2E_ADMIN_PW=… pnpm exec playwright test | **PASS 1 / FAIL 0**（25s，无 provider 栈部分旅程） |
| 移动端 390px 几何检查 | 页面溢出 0px；顶部条/表格滚动正常 |

## 当日功能快照（相对 origin/main=d0abdf4 的增量）

1. Engram 设计系统全量替换（token/组件/七域页/双主题/品牌更名）
2. 侧栏四件套：收缩（60px 窄轨+持久化+呼吸图标）、三分区、状态徽章（10s 轮询）、命令面板
3. 401 会话恢复、主题引擎（跨标签/系统跟随/sigma+mermaid 实时重渲染）
4. 移动端适配（顶部导航条 + 表格横滚）
5. 六轮视觉反馈修复记录在根 docs/plantree/frontend-polish/roadmap（0~0e）

## 开放项

1. R3：skip-to-content、移动端激活项 scrollIntoView、横滚渐隐提示
2. R4：路由 Suspense fallback 骨架化
3. toast 体系（倾向不做，保持内联反馈——根 plantree open-questions#2）
4. api-schema 生成类型与手写域类型双轨未合一
5. `e2e-design-*.mjs` 两个审计脚本属一次性工具，可择机移出 web/ 或 gitignore

## 已知风险

- 初始 chunk 若继续增长需守住 350kB 预算（当前 ~102kB 总 gzip，余量大）。
- Playwright 依赖本地栈（19180）与 E2E_ADMIN_PW；新环境跑前先起栈。
