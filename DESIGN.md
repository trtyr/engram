# Engram 设计系统

> 由 documenter 从建成的世界反写（2026-08-30，seed 40e8deb2）。权威在 token 与组件代码；本文是导航。

## 世界：墨白正统（用户钦点 Vercel/Geist 系）

**立场**：纯无彩做世界，彩色只承担语义。工具的最高形式是消失。

## Token（`web/src/index.css`）

| Token | Light | Dark | 用途 |
|---|---|---|---|
| background | `#ffffff` | `#0a0a0a` | 页面地 |
| foreground | `#0a0a0a` | `#ededed` | 正文/墨色（= brand/primary） |
| card | `#ffffff` | `#111111` | 卡面（同一地面，靠发丝线分层） |
| muted | `#f4f4f4` | `#1a1a1a` | 次级地面（code 块底/悬浮态） |
| muted-foreground | `#636365` | `#909094` | 次级文本 |
| border | `#e5e5e5` | `#262626` | **1px 发丝线（唯一分层手段，零阴影）** |
| success / warning / info / destructive | 深调 | 亮调 | 语义四色，仅状态可用 |

- 字体：Geist Variable（界面）+ Geist Mono Variable（**数据/状态/代码一律 mono**）。
- 圆角：6/8/10/12（--radius 8 基）。药丸已废弃。
- 浏览器表面已收编：选区（墨反白）、caret、细滚动条、2px 焦点环。
- 主题：系统跟随 + `engram-theme` 手动覆盖，index.html 内联引导防闪烁；`useTheme()`/`ThemeToggle`。

## 组件语言（`ui-bits.tsx` / `ui/button.tsx`）

- **Card**：`rounded-lg border bg-card`，无阴影。节标题用 `SectionTitle`（字重层级，不靠字号）。
- **Tabs / 导航选中态**：**整行反转**（bg-foreground text-background），段间发丝分隔（aria-pressed 分段控件）。
- **StatusBadge**：无底色，语义色点 + mono 状态字；`running/*ing` 态点脉冲（`.engram-pulse`）。
- **Button**：default=墨色实心（全页唯一强调位）；outline=发丝；destructive=红边红字。
- **BrandMark**：三层错位方（记忆层层留痕），纯 currentColor。
- 表格：`tableCls`（发丝行线、th 小字、`tdMono` 数字列右对齐等宽）。

## 签名交互

- **蒸馏管线条**（Memory 页）：L0→L3 层级+计数，点击直达 tab；蒸馏中箭头脉冲 + 计数。
- 图谱画布：32px 测量网格底纹；节点大小=连接数；标签 Geist 12px；边/高亮色走 token。
- mermaid：`theme:'base'` + themeVariables 从 CSS token 实时注入（亮暗随切），Geist 字体；骨架/重试态。

## 排版（`.engram-prose`）

Markdown 定制皮肤：正文 14px/1.75，**行长 4xl**（2026-09-03 从 70ch 放宽——双层限宽去一层，两侧留白 98→24px）；标题 600 字重四档；表格发丝；代码块 muted 底 mono；引用 2px 墨色左线。

## 布局

- 壳：桌面 208px 侧栏 + max-w-6xl 内容；**移动端侧栏变顶部横滚导航条**（md: 断点）。
- 间距节奏：分区 space-y-6/8 透气，区内紧凑（用户裁定：呼吸感适中，勿大留白）。
- 表格容器 `overflow-x-auto`（窄屏横滚，不挤压）。

## 禁区（craft floor 生效记录）

渐变字、彩色粗边条、硬阴影、药丸徽章、emoji 图标、mono 服装化（mono 只给数据/代码/状态）、kicker 眉题——全部未用。
