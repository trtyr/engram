# P014 · 表格化筛选（Excel 式列头筛选）

> 2026-10-04 用户拍板三点之三。状态：**planning**（先记，执行待排期）

## 问题

用户原话：「这个筛选。你能不能把那个东西做成真正的表格？然后我们所有的筛选，就像 Excel 那样，在表格上面筛选。你现在筛选，单独拿出来很难看。筛选跟列名搜索，你在这里放得很难看。」

现状：全站列表页（Memory/Logs/Assets/Credentials/Documents/Galaxy/Mcp/Projects/ProjectDetail/CodeGraph/Dashboard…）各自手写 `<table>` + **独立筛选栏**（一排 select/input 摆在表格上方），ui-bits 没有统一 Table 组件——每页 useState 一堆筛选变量，视觉与交互割裂。

## 方案骨架

1. **T001 建 DataTable 组件**（ui-bits）：列头内嵌筛选/搜索（Excel 式）、列排序、与 Pager 集成（服务端分页参数透传）、受控/非受控两用
2. **T002+ 逐页迁移**：删各页独立筛选栏 → 列头筛选；优先级按用户痛点定（首批建议 Memory + Logs，生产最高频两页）
3. 迁移完成的页面删除孤儿筛选 state 与 URL 参数同步逻辑重排

## Affected Modules

web（ui-bits + 全部列表页 features/*.tsx）

## 扩展范围：Memory 页头部布局压缩（2026-10-04 追加）

用户截图标注（两个红框）：PageHeader 行右侧空白（红框1）+ 标签页列表（红框2）。

需求（用户原话转写）：
1. 删掉「用户记忆」下的小字副标题「会话 → 蒸馏 → 原子 → 场景 → 画像，全程可溯源」
2. 标签页（会话/原子/待审/场景/画像/KV）上移，与「用户记忆」标题**同一行**
3. 「触发蒸馏」按钮（现挂 tab 行右侧 DistillBar）同移到标题行
4. 消掉 PageHeader 行右侧的空区——标题行不再是独占的两行结构

代码落点：`web/src/features/Memory.tsx:107`（PageHeader desc 删除）+ `:109-111`（tab 行与标题行合并）。注意 DistillBar 仅 sessions tab 显示、筛选器仅 atoms tab 显示（:112-113 条件渲染），合并后同行右侧区域按 tab 切换内容。

### KV 表格壳未复用 Card 样式（2026-10-04 追加）

用户原话：「KV 那里的表格样式有问题，没有复用列表的样式……它跟之前的那个其他的表格完全不是一个样子。」

实证（2026-10-04 源码对照）：同页 Sessions 列表外壳 = `<Card className="overflow-x-auto">`（圆角卡片，与 Logs 页一致）；KvPane 外壳 = 手写 `overflow-hidden rounded-md border border-border`（裸边框 div，无卡片背景）——一圆一直视觉割裂。

修法：KvPane（Memory.tsx:1349）外壳替换为 `<Card className="overflow-x-auto">`，与 Sessions/Logs 同款。一行改动，P014 开工时捎带落地。

### 会话列表：预览列吃满 + 干掉「详情」按钮（2026-10-04 追加）

用户原话：「“会话”这里的列表，最右侧留白太多了。把那里的“详细”给我去掉。点击这个“会话”，就能直接查看。留白干掉，预览加宽。」

实证（2026-10-04 源码对照，Memory.tsx Sessions 表格）：
- 预览列 `max-w-96`（384px）钳死 → 剩余宽度摊给右侧稀疏列，行右侧大片留白
- 最后一列是「详情」ghost 按钮，而展开手风琴本可整行触发

修法：
1. 预览 td 去 `max-w-96`（保留 truncate + title），让预览列吃满剩余宽
2. 行加 cursor-pointer + onClick 整行切换 openId；Checkbox 单元格 stopPropagation 防误勾
3. 删「详情」按钮列（th 空列与 td 按钮同删），手风琴 colSpan 7→6

### KV pane 说明小字删除（2026-10-04 追加）

用户原话：「AI 管道的权威精确值存储（序列号/UUID/路径…逐字保存、回读比对）。只读——写入唯一通道是 MCP memory.kv_put。这种没有任何意义的小字，给我去掉。」

落点：Memory.tsx:1326-1329（KvPane 顶部 `<p>` 说明段整段删）。同类说明小字如其他 pane 还有，登记时一律视为候选删除（Empty 提示除外——那是空态指引不是说明）。

### Settings 页：删两行小字 + JEV 归供应商 + 网页读取独立成「额外功能」页（2026-10-04 追加）

用户原话：删「系统里共有 8 个 AI 功能…」与「LLM 供应商、模型路由与记忆节律；账号/会话与 API 密钥在…」两行小字；「JEV 应该是在供应商那里，网页读取应该单独给一个页面，叫做额外功能。这个网络读取应该叫做智谱网络读取工具」。

落点与修法：
1. Settings.tsx:34 PageHeader desc 整句删；:646「系统里共有 N 个 AI 功能…」说明段删
2. JEV 决策模型配置卡片（:687/:697-）从「AI 功能」tab **挪到「供应商」tab**（JEV 本就是模型供应商配置，OpenRouter-only）
3. 网页读取配置卡片（:847-/:923「网页读取（web-reader）」）**独立成新页面「额外功能」**（侧边导航一级页），卡片名改「智谱网络读取工具」；后续同类杂项能力（AI 周边工具）归此页
4. 「AI 功能」tab 清走两张卡后重排剩余 PURPOSES 列表

拍板点：~~新页「额外功能」走侧边一级导航还是 Settings 内新 tab~~ → **已拍板（2026-10-04）：Settings 内新 tab**。

### Settings 头部布局压缩（2026-10-04 追加，与 Memory 同模式）

用户原话：「你把设置的小字去掉之后，设置跟这个标签应该是在同一行。」

即：删 desc 后，PageHeader「设置」标题与 Tabs（AI 功能/供应商/节律/危险操作/数据迁移/额外功能）**同一行**——与 Memory 页头部压缩同款模式。落点 Settings.tsx:34（desc 删）+ Tabs 行与标题行合并。

> 头部压缩模式（标题+Tabs+右侧操作同行）记为 P014 通用模式，Memory 与 Settings 两页首批应用。

### 全面清扫令：desc 小字全站清零 + 头部压缩全站应用（2026-10-04 追加）

用户原话：「全面看一下——账号与安全的小字也去掉，标签跟标题一行；MCP 改成 MCP 管理，页面小字去掉；日志下面的小字也去掉；学习、工单、待办等等，这些标题下的小字全给我干掉。」

全站 PageHeader desc 清单（2026-10-04 grep 实证，11 处全删）：

| 页 | desc 内容（删） | 备注 |
|---|---|---|
| Account 账号与安全 | 「账号管理 / 活跃会话 / MCP 密钥——…」 | +标签与标题同行 |
| Dashboard 概览 | 「记忆资产、系统活动与用量的概览」 | |
| Mcp | 「AI 接入管理——…」 | **页名改「MCP 管理」** |
| Memory 用户记忆 | 「会话 → 蒸馏 → …」 | 已登记 |
| Projects 项目 | 「项目记忆域：…」 | |
| Settings 设置 | 「LLM 供应商、…」 | 已登记 |
| Study 学习 | 「学习路线图跟踪：…」 | |
| Tickets 工单 | 「结构化问题跟踪——…」 | |
| Todos 待办 | 「不绑定项目的临时事项/灵感速记——…」 | |
| Wiki | 「Agent 维护的互链知识库——…」 | |
| Logs 日志 | 「系统里发生的一切：…」 | 筛选栏在 PageHeader children，压缩后放标题行右侧 |

头部压缩模式全站应用：标题 + Tabs/操作 同行（desc 删除后统一执行）。

## 拍板点

1. 迁移批次顺序（哪些页先进首批）？
2. 列头筛选交互形态：下拉枚举（状态类列）+ 输入框（文本列）混合，是否合意？
3. URL 同步保留与否（现有页部分筛选同步 ?query=，Deep link 是否还要）？

## Links

- ui-bits 现状：`web/src/components/ui-bits.tsx`（无 Table，Tabs/Checkbox/StatusBadge/Empty 等已有）
- Pager 已支持服务端分页（P010 日志页实证）
