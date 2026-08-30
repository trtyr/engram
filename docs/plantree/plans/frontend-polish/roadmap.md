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
0f. **登出**（用户指出「登进来不能退出，离谱」）：footer 系统区末位加登出按钮——展开态远端角落、
   收起态堆叠底部（36px，hover destructive 语义红）；客户端清 token 回登录页（后端无 logout
   端点，ams_ 随 TTL 过期——MVP 取舍已注释在代码）。验证：双态点击 → /login + token null、
   零溢出、26/26。
0g. **Memory 页五 tab 全面打磨**（用户「细致一点，每个点都细心设计」）：
   - 会话：详情改紧贴行下方的手风琴（不再跳页面底部）；触发蒸馏带 busy + 入队数反馈
     （修 api.ts 202-with-body 被 drop 的库级 bug）；空态文案去 API 黑话
   - 原子：kind 中英对照（偏好/preference 等 8 类）；溯源列（source_refs 计数 + title 明细）；
     置信度 <0.60 黄色警示；5s 无条件轮询改为仅蒸馏中轮询（省闲时请求）；200 条截断提示；
     双击编辑 hover 可见 affordance
   - 场景：卡片补「N 原子 · 相对时间」meta 行；topic 可点击展开 atom_refs 溯源
   - 画像：aspect 中英对照（7 分面）；历史改卡片内展开；回滚加 confirm；版本 chip 修
     bg-white/5 off-token 残留 → border token
   - 检索：busy/空结果 Empty/score 样式统一；L1 命中「查看」跳原子 tab（?tab= 深链扩 search）
   - vitest 28（检索面板新用例）+ e2e journey PASS
0h. **Knowledge 页打磨**（同流程）：
   - 分块预览改紧贴行下的手风琴（原来跳页面底部）；ChunksPanel 移入行内
   - 删除文档加 confirm（连分块嵌入一起删，不可恢复）
   - 上传/URL 摄取带 busy + 三色反馈（摄取中 info / 已入列 success / 失败 destructive），
     摄取中 dropzone 边框 info + 图标脉冲；accept 对齐服务端支持（pdf/docx/html/md/txt）+
     格式提示行；拖拽态由 React state 驱动（弃 classList 直改 DOM）
   - 表格新增「来源」列（URL 摄取 = link 图标 + title 全链；文件 = mime 短码）
   - 检索结果样式统一（标题 chip + #seq + FTS 徽章 + score 右对齐 mono；空命中 Empty）
   - rounded-xl → lg 统一；空态引导文案
   - 实测：手风琴/confirm 取消不删/真文件摄取反馈/双主题截图；28/28 + e2e PASS
0i. **Knowledge 主从版式**（用户「左侧目录右侧查看，像 Wiki 那种；Wiki 也难看」）：
   - 表格式列表 → 左目录（lg:w-80：标题/来源图标/相对时间/处理中脉冲点 + 标题过滤器 +
     N 篇计数）+ 右阅读区（文档头：标题/StatusBadge/来源全链/时间/删除；正文 70ch 成文）
   - 选中态整行反转（Engram 导航语言）；首篇自动选中、删除/过滤后回退首篇无空窗
   - 检索命中行可点开对应文档（document_id 直达）；分块调试信息在段落 title
   - e2e journey 选择器同步（row → 目录按钮 + 阅读区断言）；28/28 + e2e PASS
   - Wiki 主从版式为下一项
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
