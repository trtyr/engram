# 当前状态（2026-08-30 验证基线）

> 全新初始化当日实况。此前文档描述的 CI 修复、Engram 重设计等历史见 git log 与根 docs/plantree/。

## 未提交变更（重要）

本地 HEAD = origin/main = `d0abdf4`，但其上有 **38 个未提交文件**（26 改 + 15 新增 + docs）：

- `server/crates/api/src/auth.rs`——**唯一后端改动**：/jobs Accept 分流（text/html→SPA，见 api.md）
- `web/` 全套 Engram 重设计 + 侧栏四件套（收缩/徽章/命令面板/分区）+ 六轮视觉修复
- 新增：PRODUCT.md、DESIGN.md、web/src/{lib/theme.ts,lib/status.ts,components/CommandPalette*,components/ThemeToggle.tsx}
- docs/plantree/（frontend-polish 计划树）、docs/design/（审计证据）

origin CI 对 `d0abdf4` 双 workflow 绿（CI + e2e，2026-08-29）；**上述未提交内容尚未过远端 CI**。

## 当日验证记录（命令 + 结果）

| 命令 | 结果 | 时间 |
|---|---|---|
| `cargo fmt --check`（server） | exit 0 | 08-30 16:0x |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0（auth.rs 改后复验） | 08-30 16:03 |
| `cargo test --workspace` | **100 passed / 0 failed**（35 套件；此后 server 零改动） | 08-30 16:07 |
| `pnpm exec tsc --noEmit`（web） | 0 errors | 08-30 17:0x |
| `pnpm run lint`（web, oxlint） | exit 0，**0 warnings** | 08-30 17:0x |
| `pnpm test`（web, vitest） | **26/26**（5 文件，含 CommandPalette 5 新用例） | 08-30 17:0x |
| `pnpm run build`（web） | exit 0；初始 JS gzip ~93kB + CSS 8.5kB | 08-30 17:0x |
| `pnpm exec playwright test`（e2e journey，本地栈） | **PASS 1 / FAIL 0**（终树复跑，25s） | 08-30 17:1x |
| OpenAPI 活体（:19180） | 55 路径（GET26/POST24/PATCH1/DELETE4） | 08-30 17:0x |
| 运行库表清点 | 19 业务表 + _sqlx_migrations | 08-30 |

本地设计验证栈：:19180（am_design_audit 库，种子数据齐全）当日全程可用。

## 已知开放项

1. 未提交批次待分块 commit + push + 盯 CI（沿用既定工作流）。
2. `cargo audit`：rsa 孤儿 + 4 transitive 提示——已接受（见 tech-stack.md）。
3. 前端 R3/R4 打磨项（skip-link、移动端 scrollIntoView、路由骨架屏）——见根 docs/plantree/frontend-polish roadmap。
4. codegraph CLI 版本钉在 Dockerfile（1.5.0），本地 homebrew 版本可能领先——行为差异未审计。

## 健康快照

- 测试：后端 100 + 前端 26 + e2e 1，全绿。
- 无已知运行时缺陷；当日新增功能均经 Playwright 几何/像素验证。
