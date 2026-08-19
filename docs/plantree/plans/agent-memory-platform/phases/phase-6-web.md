# Phase 6 — Web 控制台

**目标**：七域完整管理界面，消费既有 API，生产质感。前后端单端口交付形态在本阶段定型。

## 前置

Phase 2–5 的 API 已稳定（OpenAPI 快照为准）。

## 交付物（按子批推进）

### 6a 壳与骨架

- [ ] 登录页（管理员密码）→ 会话管理 → 布局壳（侧边栏七域 + 路由 + 暗色主题默认）
- [ ] `lib/api.ts` + OpenAPI 类型生成流水线（CI 校验生成物与后端同步，漂移即失败）
- [ ] Dashboard：统计卡 + 近期 jobs + LLM 用量图（聚合自 /llm/usage）

### 6b Memory 域 UI

- [ ] 会话列表→详情（轮次渲染 + 蒸馏产物反向链接 + 状态）
- [ ] Atoms 表格：kind/status 过滤、行内编辑、supersede 操作、needs_review 人审队列
- [ ] Persona 分面视图 + 版本 diff + 回滚；蒸馏触发按钮 + 进度（SSE 任务组件复用）

### 6c Knowledge 域 UI

- [ ] 文档表格（状态徽章/进度条 SSE）+ 上传/URL 对话框（拖拽）
- [ ] 文档详情：元信息 + 分块预览 + 失败块提示；检索试验场（高亮结果）

### 6d Wiki 域 UI

- [ ] 页面浏览器（type 过滤/搜索）+ Markdown 渲染（wikilink 跳转 + mermaid）
- [ ] 人工编辑器（编辑/预览双栏，保存即版本化）；proposal 审核队列（diff 视图）
- [ ] 图谱视图（sigma.js，type 着色/邻接高亮）；Ingest 面板；Lint 报告页

### 6e CodeGraph + Jobs + Settings UI

- [ ] CodeGraph：项目卡片（status/stats）、注册/同步、查询试验场（结果代码渲染）
- [ ] Jobs：表格过滤 + 详情事件时间线（SSE 实时）+ 重跑
- [ ] Settings：providers CRUD+测试、路由规则编辑、API keys 管理、通用参数

## 出口标准

1. 浏览器（对 compose 真栈）完成旅程：登录 → 上传文档到 ready → 写入会话触发蒸馏到画像更新 → wiki ingest 到页面出现并图谱可见 → codegraph 注册到查询返回 → jobs 页看到全程事件——每步 UI 状态与 API 一致
2. vitest 覆盖关键组件（atoms 表格操作、persona diff 渲染、SSE 进度组件、编辑器保存）
3. 类型生成流水线 CI 强制（后端 API 变更未重新生成 → CI 红）
4. 每页空态/加载态/错误态齐备；Lighthouse 基本可达性检查通过
5. `vite build` 产物由后端嵌入单端口服务验证；Phase 0–5 出口标准依然全绿

## 关联

- 设计：[frontend](../topics/frontend.md)
- 标准：ui-standards（状态覆盖/层级/响应式）
