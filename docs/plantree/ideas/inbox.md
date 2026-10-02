# Ideas Inbox

低承诺想法池，晋升前不算数。

## 2026-10-02 · 学习系统：学习路线图跟踪器（✅ 已晋升 P007，见 plans/007-study-learning-tracker/；关联 EN-33）

**用户原话（意图保真）**：「除知识沉淀之外的一整套跟踪系统，还不是 todo。把我要学的东西、学的领域记录一下，现在学了什么东西记录一下，之后学什么东西记录一下。跟踪一下我学了什么东西，同时还能跟 wiki 挂钩。」——用户自述「效果可能很神奇，但说不清楚」。

**当日具象化草案（chat 中提出，未确认对味）**：

- 模型三层：领域 track（要学的领域，如 RAG）→ 节点 item（知识单元，状态机 待学/进行中/已学）→ 节点挂 wiki 页（[[互链]]；知识沉淀照旧归 wiki/harness，不在本系统）
- 交互画面：「我 RAG 学到哪了、下一步学啥」一查即出；节点跳知识页
- 可选维度（未拍板）：时间线记录（何时学了什么）、掌握度维度（用户未答）

**承载两条路线（未拍板）**：

- A. 轻量新域 study：两张小表（track+item）+MCP 工具面+简单前端页——结构化查询/agent 维护体验最好（当日倾向）
- B. wiki 页承载：路线图=markdown checklist 页——零表结构，但跨领域总览/状态查询弱

**背景更新（相对 EN-33 原始表述）**：harness（P004-T010）落地后「知识沉淀自动化」环节已解决——本需求的独特价值聚焦在「学习过程的路线图跟踪」。

**晋升触发**：用户能回答「模型对不对味 + 承载 A/B + 要不要掌握度」三问时，晋升为 plan（候选 P007）。

## wiki 前端 agent-first 重设计（2026-10-02 侦察）

背景：P004 织入流水线退役后 wiki 维护已 agent-first（harness 12 工具+MCP 28 actions），但前端 Wiki.tsx 还是人肉维护形态（1016 行：tree+编辑器+graph+inbox/ops 五 tab）。30+ HTTP 端点里版本管理/repair/query-gaps/duplicates/promotions 记录均无 UI；harness 干活（ingest 下发→job→report）在 wiki UI 完全不可见，要去 Jobs 页翻。人的核心职责（人审 reviews/proposals）面板较弱。设计方向：三层心智（知识层/检索层/Agent 层）+人审中心强化+ingest 语义改「喂给维护 Agent」+补版本/体检缺面板。已拍板（2026-10-02）：人审**整体移除**（非保留不升级——用户明确要求系统里人审相关全删）；晋升为 P008（plans/008-wiki-frontend-agentfirst），侦察结论与删除边界已入 README。

## todos 前端：详情查看+markdown 渲染（2026-10-02 侦察）

用户报：①todo 没法像工单一样点击查看详情 ②markdown 渲染没做好。侦察（Todos.tsx 480 行 vs Tickets.tsx 对照）：todos 现状是纯行内操作（勾选/到期清理/归档/删除），无点击进详情、无 markdown 渲染；tickets 已有现成模式——selected 状态+TicketDetail 主从布局+WikiMarkdown 组件渲染详情。且 react-markdown 依赖已在 package.json，WikiMarkdown 组件现成——todos 补齐零新依赖。修法方向：抄 TicketDetail 主从模式（点行进详情）+详情内 content/note 用 WikiMarkdown 渲染。待拍板：立 P009 还是并入其他前端工作。
