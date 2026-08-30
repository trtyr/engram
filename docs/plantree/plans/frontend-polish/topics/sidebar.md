# 侧边栏评审记录（2026-08-30）

用户三问的完整答案存档。评审对象：`web/src/App.tsx` Shell（Engram 版）。

## 应有 vs 现有

| 功能 | 判定 | 说明 |
|---|---|---|
| 寻路导航 | ✅ 有 | 七域 NavLink，反转选中，aria-current |
| 系统状态总览 | ❌ 缺 | Jobs 失败数/蒸馏活动不浮现——「异步是常态」原则在全局层无落点 |
| 全局检索入口 | ❌ 缺 | 跨域检索只在 Dashboard；无 `/` 或 Cmd+K（审计 Alex 红旗未解） |
| 收缩能力 | ❌ 缺 | **用户点名**。固定 208px，无 icon-only 窄轨 |
| 分区语义 | ❌ 缺 | 七项平铺，无 资产域/系统 分组 |
| 身份/版本/主题 | ✅ 弱 | v0.1.0 硬编码 + ThemeToggle |

定性：视觉语言（反转/发丝/墨色）成立，但内容密度只有「仪器面板」该有的三分之一——好壳子没装仪表。

## Bug 清单（实锤）

| 级别 | 问题 | 根因位置 |
|---|---|---|
| P1 | 切主题后 sigma 图谱不跟随（边色滞留旧主题，暗边亮底隐形） | WikiGraph useEffect deps 无 theme；themeColors() 挂载时一次性取值 |
| P1 | 切主题后 mermaid 不跟随（SVG 皮肤滞留） | MermaidBlock 同理，renderMermaid 的 themeVariables 一次性注入 |
| P1 | **会话中途失效无全局恢复**（实测：token 失效后站内导航停留原页、永久「加载中」，不跳登录，需手动刷新触发 mount 探活） | api.ts 401 只 clearToken+throw；App 只在 mount 探活一次，无全局 401 重定向 |
| P2 | 无 skip-to-content（实测：键盘第一个焦点是主题按钮，要跨 ~8 个元素才到内容区） | App.tsx 壳无跳过链接 |
| P2 | 移动端激活项可能滚出视口（7 项 × ~90px > 390px，无 scrollIntoView、无渐隐提示） | App.tsx 移动导航条 |
| P2 | prefers-color-scheme 变化不监听（无手动覆盖时不实时跟随系统） | index.html 引导脚本一次性读取 |
| P3 | v0.1.0 硬编码；且移动端完全不显示版本 | App.tsx 侧栏底部 |
| P3 | 横滚导航滚动条在两种底色上偏隐形 | 全局 scrollbar 样式 |
| P3 | 桌面导航双重间距：`gap-1` 与 `md:space-y-0.5` 叠加（实际 6px，非设计值） | App.tsx nav class |
| P3 | 多标签页主题不同步（无 storage 事件监听，A 标签切主题 B 标签滞留） | lib/theme.ts |
| P3 | 导航图标 opacity-90 常量——激活/非激活无透明度差分（层级只靠反转背景） | App.tsx 图标 |

## 观察项（记录不立项）

- 移动端 main 为内部滚动容器（非 body 滚动）——iOS Safari 内部滚动手感略逊原生；归 R3 一并观察。
- api.ts 401 后 throw 的错误在部分页面（如 Jobs）表现为永久 Spinner 而非 ErrorBox——错误传播路径有分叉，401 全局处理落地时一并收口。

## 实测排除（不是 bug）

- 切页后 main 滚动残留——实测复位（settings→0 / wiki→0，2026-08-30 Playwright 实测）
- aria-current 缺失——实测存在（NavLink 自带）
- 双 ThemeToggle 并存——`md:hidden` / `hidden md:flex` 互斥，不并存
