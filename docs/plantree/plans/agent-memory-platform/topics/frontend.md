# Topic — Web 前端

React 19 + Vite + TS + Tailwind + shadcn/ui + Zustand + TanStack Query。类型全部从 OpenAPI 生成。

## 信息架构（侧边栏七域）

| 区域 | 关键页面 |
|---|---|
| Dashboard | 统计卡（atoms/scenarios/文档/wiki 页/任务）· 近期 jobs · LLM 用量图 |
| Memory | 会话列表→详情（轮次+蒸馏状态）· Atoms 表格（kind/status 过滤、编辑、supersede）· Persona 视图（分面+版本 diff+回滚）· 蒸馏触发按钮 · 人审队列 |
| Knowledge | 文档表格（status 徽章+进度）· 上传/URL 对话框 · 文档详情（分块预览）· 检索试验场 |
| Wiki | 页面浏览器（type 过滤）· Markdown 编辑器（人工编辑）· 图谱视图（sigma.js）· Ingest 面板 · Lint 报告 · proposal 审核队列 |
| CodeGraph | 项目卡片（status/stats）· 注册/同步 · 查询试验场（explore/callers/... 结果渲染） |
| Jobs | 任务表格（kind/status 过滤）· 详情：事件时间线（SSE 实时）· 重跑按钮 |
| Settings | LLM providers CRUD+测试 · 路由规则编辑 · API keys 管理 · 通用参数（预算/并发/触发策略） |

## 数据与状态

- 服务端状态：TanStack Query（统一 queryKey 约定、SSE 事件驱动 invalidate）
- UI 状态：Zustand（主题、侧边栏、编辑器草稿）
- API client：`lib/api.ts`（fetch 封装，错误体统一 toast）；类型 `lib/api-types.ts` 生成物不手改

## 关键交互细节

- Markdown 渲染：react-markdown + mermaid + wikilink 高亮（点击跳页）
- 图谱：sigma.js + graphology，节点着色按 page_type，出边高亮
- 长任务：全部走 jobs 进度组件（复用事件时间线）
- 空态/错误态/加载态每页必备（ui-standards 基线）
- 暗色主题默认，shadcn 主题变量实现

## 构建集成

`vite build` 产物由后端嵌入（rust-embed）单端口服务；开发时 Vite proxy 到 :8080。
