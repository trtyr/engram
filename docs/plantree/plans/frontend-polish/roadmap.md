# Roadmap

## In Progress

（无——R1/R2 已落地，见 Done；R3/R4 待用户排期）

## Next

### R3 移动端与可达性
- 激活导航项 scrollIntoView（切页时保证反转高亮可见）
- 横滚导航渐隐边缘提示（mask-image 渐变）
- skip-to-content 链接（键盘用户跨 8 元素问题）
- iOS 内部滚动手感观察

### R4 杂项（剩余项）
- 路由级 Suspense fallback 骨架化（min-h-40 空白闪一下）

## Done

### R1 P1 修复（2026-08-30 落地）
1. **主题同步**：theme.ts 事件广播（engram-theme-change）+ WikiGraph/MermaidBlock useThemeTick 重渲染。
   验证：页内切主题双截图 magick 对比，图谱 99.99% / mermaid 99.92% 像素翻转。
2. **会话中途失效全局恢复**：api.ts 401 → engram-auth-expired 广播（有 token 才发）→ App 监听回登录页。
   验证：篡改 token 站内导航 2s 内回 /login。
3. **/jobs Accept 分流**（D-001 后端授权）：bearer_auth 中间件顶部 text/html+/jobs → SPA。
   验证：curl 三态（text/html→SPA / 无 Accept→401 / JSON→401）+ 浏览器硬刷新 SPA 挂载。
   server 侧：auth.rs + fmt/clippy/test 100 全绿。

### R2 侧边栏功能补全（2026-08-30 落地，用户钦点）
0. **落地后回归修复**：长内容页（Dashboard）侧栏底部区滚出视口——移动端适配时 `h-screen`→`min-h-screen`
   使桌面失去高度约束，长内容把外层撑高、aside 拉伸、footer 沉底。修复：`md:h-screen`（桌面锁高 +
   main 内部滚动，移动端保持 body 滚动）。6 路由 + 移动端全验证 footer 可见。
0b. **收起态布局修复**（用户视觉反馈「非常奇怪」）：实测两处——头部 brand 容器被 justify-between 挤成
   5px（logo 压扁）、尾部主题按钮溢出侧栏边界 10px（38~66px vs 56px 宽）。修复：收起态头/尾两行
   `md:flex-col md:justify-stretch md:gap-1` 纵向堆叠居中 + brand `shrink-0`。验证：溢出元素 0、
   logo 20px 居中、展开 208px 回归正常、移动端 0px 溢出。
0c. **收起态脉冲点挤位**（用户视觉反馈）：蒸馏脉冲点的 ml-auto 在窄轨里把居中图标推偏。修复：收起态
   点隐藏、图标自身呼吸（engram-pulse 升级 @utility 以支持 md: 前缀变体），rail 标签附「· 蒸馏中」。
   验证：图标居中偏差 0.0px、点 display=none、图标 animation=engram-pulse。
0d. **收起态比例重设计**（用户反馈「图标太小」）：56px 轨 + 16px 图标（占比 0.29）过弱 →
   **60px 轨 + 20px 图标（占比 0.33，Vercel/GitHub rail 范式）**，行高 36px，品牌印记 24px，
   头/尾按钮收起态 p-2 + 图标 20px（ThemeToggle 加 iconClass 透传）。验证：居中 0.0px、
   展开 208/16px 回归不变、26/26、lint 0。
0e. **内容区留白收敛**（用户反馈「留白太多，收缩后更大」）：根因 max-w-6xl(1152) 封顶 + px/py-8——
   收缩释放的宽度全变成居中空白（1512 屏：展开 ~84px/侧 → 收起 ~158px/侧）。改为
   **max-w-[1440px] + 24px 边距**：1512 屏两种状态边距恒为 24px（收起仅 +6px），1920 超大屏
   居中封顶防表格无限拉伸。验证：内容宽 1304(展开)/1440(收起)、26/26、lint 0。
1. **收缩**：208px ↔ 56px icon 轨，localStorage(engram-sidebar) 持久化，title 提示，动画 200ms。
2. **状态徽章**：useSystemStatus 10s 轮询（页面隐藏跳过）；Jobs 项 failed+dead 计数芯片（收起态角标点）、
   Memory 项蒸馏中脉冲（kind ∈ extract/extract_atoms/arbitrate/organize/consolidate）。
3. **全局检索**：CommandPalette（Cmd/Ctrl+K、/、侧栏按钮）——POST /search，↑↓ 选择 Enter 跳转 Esc 关闭，
   挂载即重置；App 侧 openedAt 派生（路由变化自然关面板，无 effect-setState）。
4. **分区语义**：首屏｜资产域｜系统（mono 小节标签；收起态隐藏标签、分组间发丝线）。
5. 顺带修复：导航双重间距（去掉 space-y 保留 gap）、图标 opacity-90 常量、版本号单一来源
   （vite define 读 server/Cargo.toml workspace version）、跨标签主题同步（storage）、
   系统偏好实时跟随（matchMedia，无手动覆盖时）、移动端检索按钮。
   验证：Playwright 全项（徽章 5/脉冲 1/56px 刷新持久/面板检索命中/Esc/401 恢复/jobs SPA）+
   vitest 26/26（CommandPalette 5 用例新增）+ e2e journey PASS + 移动端 390px 无溢出。
   截图：docs/design/screenshots/r12-*.png（8 张：亮暗×展开收起、面板、图谱/mermaid 双主题、移动端）。

## Deferred

- toast 系统想法（open-questions#2，倾向保持内联反馈）
