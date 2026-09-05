# 当前状态（2026-09-04 验证基线）

> 2026-08-30 初始化，09-01/09-03/09-04 多次全面更新。

## 一句话状态

vitest **45**（9 文件）/ oxlint 0 警告 / tsc 0 / build 0 / 入口 bundle **285.60 kB**（gzip 91.81，预算 350 内）。
origin/main 8af183e 产品更名 Engram 完成。Wiki 页 09-03 完成 Obsidian IA 重做（roadmap 0w，28 项审计全修）。

## 2026-08-30 基线以来的前端大事记

1. **记忆星系 → 圈子 → 独立页**：先以 tab 落地（8bfd330），改名圈子（3e9f95b），
   最终拆独立页 /circle（36342e8）——一坐标系脱离一架梯子
2. **人审队列 tab**（d7c7fdf）：通过/取代/丢弃 + 批量
3. **re-embed 横幅**（ae4daad）：向量缺失诊断与修复入口
4. **编辑能力 UI**（534d701）：原子双击编辑/敏感开关/历史抽屉 + 画像编辑钉住 + 实体摘要手编
5. **画像三件套**（1d9ee28）：右滑历史抽屉 + 句子级 LCS diff + 证据链跳场景
6. **deep 清空 UI**（bfa2e3f 部分）：危险区确认短语门禁
7. **登录态根修**（b5ac042）：探活 401-only（5xx 不再误杀）
8. **e2e 自清**（a7ae4f4）：journey 收尾清 agent/实体/key
9. **测试隔离**（399deab）：E2E_BASE 必填拒跑 + 一次性栈脚本 + 快照差分自清
10. **设置节律 tab**（c1f877f）：cron 心跳三态 / 积压年龄 / crontab 安装向导 / 节律事件流
11. **Wiki+Knowledge 合并**（2026-09-02）：/knowledge 并入 /wiki 前缀，前端融合一个 Wiki 页（文档 tab 默认，接管上传/URL/阅读/检索），删侧栏「知识库」项（DocumentsPane 组件被 Wiki 挂载）；图谱 Obsidian 化（hover 邻居高亮/拖拽/缩放/边权重/位置缓存）
12. **设置页 AI 配置体系重构**（2026-09-02，ac55a6d..2397d53 八连提交）：先看功能再配 API（AI 功能页纯选）、供应商真 label 表单 + 类型下拉、批量吊销、全站下拉框美化（去原生箭头 + 自定义 chevron）
13. **Wiki Obsidian IA 重做**（2026-09-03，1edff15..a3b6a2d，roadmap 0w 28 项审计全修）：目录树（folder 层级，折叠 localStorage 持久化，role=tree 语义）+ 树/图双视图切换 + 收件箱/运维二级面板；布局骨架重做（视口实算/状态提升切视图不丢/分割线拖拽 220-480px/?page= 深链自动展开 folder）；双链渲染三连修（递归 withWikilinks 深入行内 children 覆盖标题/列表/表格 / 404 显性提示不再静默刷树）；阅读区排版 70ch→4xl 放宽；Tabs 脏竖线改发丝网格；新增 e2e wiki-ia.spec.ts（折叠持久化/URL 写回/深链断言）

14. **MCP 管理页**（2026-09-05）：/mcp 九页，管理台布局（顶部状态条一行收口：状态灯 + 端点 + 协议/版本 + 总开关）——域 Tabs 逐域查看工具（域归属由后端 McpToolInfo.domain 同源提供，按工具名前缀归域；目前记忆域，未来 Wiki/CodeGraph 接入即新增 tab）+ 拨杆开关列表（语义徽标、说明收 tooltip、启用 N/M 计数）；服务开关（关闭 = /mcp 503）、工具粒度开关（停用 = 对 AI 隐身 + 调用拒，PUT /settings/mcp 覆盖式 disabled_tools）；密钥管理在设置页 Keys tab（七 scope 选择器），接入指导按用户决策不做在管理页。

## 当日验证

| 命令 | 结果 |
|---|---|
| pnpm run lint（oxlint） | 0 警告 |
| pnpm exec tsc --noEmit | exit 0 |
| pnpm test | 45 passed / 9 文件 |
| pnpm run build | exit 0；入口 285.60 kB（gzip 91.81） |
| CI（f8e1031） | e2e + CI FAIL（支出限额，未启动） |

## 已知前端未了项

- bundle 预算 350kB 内（当前 285.23，sigma/mermaid/cytoscape 均在 lazy chunk）——持续达标
