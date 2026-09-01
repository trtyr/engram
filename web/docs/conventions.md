# 约定

> 2026-08-30 依据源码、DESIGN.md 与测试实践归纳。

## 设计纪律（来自 DESIGN.md，写 UI 前必读）

- **彩色只留语义**：success/warning/destructive/info 四语义 + 图谱数据编码色；禁止品牌彩。
  off-token 色（green-400/blue-500 之类）不允许出现——lint 之外的口头门禁，审计曾清过 28 处。
- 发丝线（border-border）替代阴影；选中态 = 整行反转（bg-foreground text-background）。
- 数字列一律 `tabular-nums`/`tdMono`（等宽 + 表格数字）。
- 禁区：kicker/眉标、渐变文字、玻璃拟态装饰、硬偏移阴影、pill 徽章、emoji 图标、mono 当戏服。
- 新页面必须双主题都过目；一次性取色的组件订阅 useThemeTick。

## 组件与状态

- 页面（features/）只编排，可复用逻辑沉到 components/lib。
- setState 不进 effect（事件驱动或派生）——oxlint `react/set-state-in-effect` 零警告是门禁。
- 主题/认证等跨组件信号用 window CustomEvent（engram-theme-change / engram-auth-expired），
  不引第三方 store。
- 危险操作（擦除/删除/吊销/重加密）必须 confirm() + destructive 变体按钮 + 危险区隔离。

## 选择器契约（e2e 稳定性）

- 表单控件必须 `<label htmlFor>`（getByLabel）；按钮用中文 accessible name
  （getByRole('button', {name:'原子', exact:true})——exact 防 PipelineStrip 同名子串撞车）。
- 卡片结构变化（如 rounded-xl→rounded-lg）会破坏 e2e 的 div.rounded-* 选择器——改组件同步改 spec。

## 测试策略

- vitest（jsdom + testing-library）：组件行为面（交互、渲染断言）；新组件逻辑同步补测
  （如 CommandPalette 的聚焦/检索/键盘选择/Esc/错误态）。
- Playwright journey：全链路语义回归，无 provider 自动降级部分旅程。
- 视觉验证脚本（e2e-design-shots/metrics）：大改后双主题截图 + 几何/像素断言。

## Git

- Conventional Commits 中文描述；按主题分块。
- bundle 预算：初始 <350kB gzip；新重库必须进 lazy 边界。
