# Baseline

项目级基线的权威来源是 [docs/](../../README.md)（全栈档案），本目录只放规划用的导航性摘要，不复制内容。

- 模块地图 → [docs/architecture.md](../../architecture.md)
- 运行/部署/验证命令 → [docs/run-and-deploy.md](../../run-and-deploy.md)
- 设计系统（Engram 墨白正统）→ [DESIGN.md](../../../DESIGN.md)、产品事实 → [PRODUCT.md](../../../PRODUCT.md)
- 前端设计审计证据 → [docs/design/](../../design/audit.md)（audit.md + screenshots + metrics）
- 后端基线 → [server/docs/](../../../server/docs/README.md)

关键约束（规划时必须遵守）：

1. 后端 API 契约稳定。~~前端重设计类工作 `server/` 零改动~~ → **2026-08-30 放宽**（D-001）：为完善功能的后端改动经用户授权可行，须保持全门禁绿；例外仍需显式决策。
2. 全部质量门禁保持绿：lint / vitest / build / Playwright journey（本地无 provider 栈）。
3. Engram 视觉世界已锁定（用户钦点）：无彩双主题、发丝线、反转选中、彩色仅语义——见 DESIGN.md 禁区清单。
