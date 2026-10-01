# P004 Open Questions

## 未决

（空——全部已决，见下）

## 已决归档

### Q001 · 滞留恢复机制（✅ 已决 2026-10-01）
- **结论：B+A**——re-embed 上 MCP（已并入 T007 工具面盘点，独立任务取消）+ 文档化「重新 document_add 同 URL 触发 K1 自愈」姿势（落 T008 手册）；C 后台扫描不做（仅三篇滞留，P001「无观察失败不加 watcher」先例）。落地 = T002（收尾确认）。

### Q002 · URL 正文提取增强（✅ 已决 2026-10-01，用户拍板）
- **结论：接入智谱 web-reader MCP**（用户原话：链接的获取走它的接口）。接口已实测：`https://open.bigmodel.cn/api/mcp/web_reader/mcp`，工具 `webReader`，返回 title/description/url/content 结构化 markdown + 链接表，对 JS 渲染页同样有效。key 已存 credentials `zhipu/web_reader_key`。落地任务 = T005（后端抓取层）+ T006（前端设置页）。本地 readability 方案弃选。

### Q003 · EN-31 导入降级（✅ 已决 2026-10-01，长任务授权默认拍板）
- **结论：实现导入降级**——嵌入瞬态故障期间 submit 正常返回，文档 FTS 先可检索，恢复后补嵌（与现有块级 embed_failed 降级语义一致，见 T001 守卫终态）；「熔断期间是否直接 ready」的实现细节随 T001/T002 定。文档化硬依赖理由随之作废。

### Q004 · wiki 摄取 Agent 化（✅ 已并入 Q005/T010）
- 用户 2026-10-01 提出 wiki 背后应有完整 Agent——已被 Q005（C+ Agent Harness）完整覆盖，本条不再独立存在。harness 的自主递归追链接（深水区自动化）见 roadmap Deferred。

### Q005 · 后端自动织入流水线去留（✅ 已决 2026-10-01，用户拍板 C+）
- **结论：移除死的确定性织入流水线，由「wiki 维护 Agent Harness」接管 ingest 入口**（用户原话：「其中一个是其实还是有后台的，只不过后台是一套完整的 agent loop，是一套完整的 agent harness。这个 Agent 专门服务于维护我们这个 wiki……我们可以给他一个链接，也可以给他一个内容。如果给他链接，他可以通过我刚才给你的那个接口，利用工具自己去上网搜索；同时给他工具，让他能够自己操作这个 wiki，并自己去维护。我们之后这一整套东西，背后全部是有一个 Agent 的」）。
- 形态：ingest 入口语义保留（喂链接或内容），内部实现从确定性 LLM 流水线换为 agent loop——harness 持有 web_reader（智谱）+ wiki 全操作工具，自主抓取/提炼/建页/互链/维护，purpose 作为其 system prompt 素材。落地 = T009（移除旧实现）+ T010（harness）。
- 边界：**保留** = write_page、document_add 分块嵌入（检索层）、lint/lint_deep/graph/reviews/merge 等 agent 显式工具。wiki_sources 存量表按「迁移只增不改」留表停写（或标记废弃，不 drop）。
- 要点：向量的定位是检索层（不是知识层）；外部 Agent 的 MCP 全量工具（T007）与后端 harness（T010）双层并存——手动维护与无人值守维护两种形态。
- 四项实施拍板（同日）见 roadmap T010：①Purpose::WikiAgent 独立档位 ②破坏性工具给 harness 全审计 ③document_add 自动接力 ④全局单消费者队列。
