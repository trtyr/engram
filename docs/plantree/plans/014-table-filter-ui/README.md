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

## 拍板点

1. 迁移批次顺序（哪些页先进首批）？
2. 列头筛选交互形态：下拉枚举（状态类列）+ 输入框（文本列）混合，是否合意？
3. URL 同步保留与否（现有页部分筛选同步 ?query=，Deep link 是否还要）？

## Links

- ui-bits 现状：`web/src/components/ui-bits.tsx`（无 Table，Tabs/Checkbox/StatusBadge/Empty 等已有）
- Pager 已支持服务端分页（P010 日志页实证）
