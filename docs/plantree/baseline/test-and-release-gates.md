# 门禁与红线

> 详情见档案「测试与门禁」01a0e6ec-098a-7531-8d5e-90a6c8cd073b。改代码前必读。

## 提交门禁（README:172-178 与 CI 一致）

```bash
# server
cd server && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
# web
cd web && pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build
```

## 事故换来的硬红线

1. **本地 journey 唯一合法入口 `scripts/e2e-local.sh`**——journey.spec.ts 缺 `E2E_ADMIN_PW` 直接 throw，`E2E_BASE` 缺省回落本地栈口（真防线是入口脚本锁死本地地址；三次误删生产库教训）
2. 改 MCP 工具面必须过 `tests/golden/mcp_surface.json` 快照；新增 action 必须登记读写分类（dispatch.rs:718 护栏）
3. 前端产物被 rust-embed 编译期嵌入——前端改动生效必须重建 `web/dist`；`assetsDir` 必须 `'static'`
4. 迁移只增不改（sqlx 单向）；加迁移必同步 `migrations_test.rs` 版本断言（当前 75）
5. 改检索必跑 `scripts/wiki-benchmark`（Hit@5/MRR@5 降级立刻可见）
6. 凭据红线：token/密码只写变量名；credentials 值永不进日志/文档/记忆
7. 生产禁裸崩：`deny(clippy::unwrap_used/expect_used)`（例外需注明理由）
