# 当前状态（2026-08-30 验证基线）

> 本页是全栈快照；分栈细节：[server](../server/docs/current-state.md)、[web](../web/docs/current-state.md)。

## 一句话状态

origin/main `ffc5568` **双 workflow 绿**（CI 33303163978 + e2e 33303163994，2026-08-30）。
Engram 重设计 + 侧栏四件套 + /jobs 分流 + 三层文档已全部落地推送（7 个主题提交）；
中途 docker/e2e 曾因版本注入的构建上下文假设双红一轮，已修（Dockerfile 补 COPY + vite 防御）。

## 当日验证矩阵（终树）

| 栈 | 命令 | 结果 |
|---|---|---|
| server | cargo fmt --check | exit 0 |
| server | cargo clippy --workspace --all-targets -- -D warnings | exit 0 |
| server | cargo test --workspace | 100/100（35 套件） |
| web | pnpm run lint（oxlint） | exit 0，0 警告 |
| web | pnpm exec tsc --noEmit | 0 errors |
| web | pnpm test（vitest） | 26/26 |
| web | pnpm run build | exit 0，初始 ~102kB gzip |
| e2e | playwright journey（本地栈 :19180） | PASS 1 / FAIL 0 |
| 事实 | OpenAPI 活体 / 迁移 / 表 | 55 路径 / 14 迁移 / 19 业务表 |

## 未提交批次内容（待提交清单）

1. `server/crates/api/src/auth.rs`——/jobs Accept 分流（唯一后端源码改动）
2. `web/`——Engram 设计系统 + 七域页重做 + 壳功能（收缩/徽章/面板/分区/主题引擎/401 恢复/移动端）
3. `PRODUCT.md` / `DESIGN.md`——产品与设计事实
4. `docs/design/`（审计证据）、`docs/plantree/`（frontend-polish 规划树）
5. 本轮三层项目档案（docs/ + server/docs/ + web/docs/ 全量重写）

## 开放项

| 项 | 位置 |
|---|---|
| 未提交批次分块 commit + push + 盯 CI | 既定工作流 |
| R3/R4 前端打磨（skip-link/scrollIntoView/骨架屏） | docs/plantree/frontend-polish/roadmap |
| cargo audit 接受项（rsa 孤儿等） | server/docs/tech-stack.md |
| api-schema 生成类型与手写类型双轨 | web/docs/current-state.md |
| Docker 部署未启用（开发期方针） | docs/run-and-deploy.md |

## 交接提示

- 本地验证栈可能仍在 :19180（am_design_audit 种子库，密码 design-audit-pw）——用前 curl /ready。
- 设计证据（22+ 截图、审计报告、色彩/度量 JSON）在 docs/design/。
