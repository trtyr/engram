# P009 roadmap

## Done

- [x] **导航重组**：学习移入「资产域」组；「任务」入口移除 ✓ 2c5423a
- [x] **任务并入日志**：日志页「后台任务」独立区块（状态筛选/事件时间线/重跑）；删 Jobs.tsx 与 /jobs 路由 ✓ 2c5423a
- [x] **日志 7 天窗口**：默认 since=now-7d，新增近 1/7/30 天/全部切换 ✓ 2c5423a
- [x] **长内容点击展开**：LongText 组件（>140 字截断+展开/收起），用于日志消息/任务错误/事件 ✓ 2c5423a
- [x] **已完成计数修复**：原先受展开门控致初始 0 → 改为始终拉计数 ✓ 2c5423a
- [x] **账号与安全拆三标签**：账号管理 / 活跃会话 / MCP 密钥 ✓ 2c5423a
- [x] **网页读取并入 AI 功能**：移除独立 tab，挂到 Routing 尾部 ✓ 2c5423a
- [x] **生产 codegraph 修复**：compose 注入 AGENT_MEMORY_DATA_DIR=/app/data（根因：HOME fallback 写容器可写层，重建即丢）+启动日志可见化+测试+生产 20 条 path 迁移+4 项目重建 ✓ 72003f6/dbef672/cb7bf7d

## In Progress

（空——收官门禁中）

## Next

（空）

## Deferred

- 16 个 client_upload 型 codegraph 条目：产物服务端无备份（本次容器重建永久丢失），需客户端本机重新 index 后 upload
