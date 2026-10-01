# P004 Roadmap

> 任务身份/状态/顺序的唯一权威。破案细节见 README「破案结论」。方向转折见 README「转向」。

## 执行序（夜间长任务 2026-10-01→02）

跨线总序：**T001 → T007 → T005 → P005 日志六任务 → P006 错误处理五任务 → T009 → T010 → T006 → T002 → T003 → T004 → T008**。依赖依据：T007 统一工具层是 T010 harness 的工具全集来源；T005 web-reader 是 harness 工具之一；P005/P006 提供 harness 的审计与归因基础设施；T009 拆旧后 T010 接管 ingest。

## In Progress

（空——夜间长任务 2026-10-02 收官）

## Next

（空——全部任务已结）

## Deferred

- **harness 深水区·自主递归追链接**（原 Q004 残余）：webReader 返回链接列表后 agent 可手动追；自动化（预算控制/深度限制/循环防护）在 T010 落地实测后再立项。
- **T010 真实 demo 补验**：harness 全链路已验证（mock 集成 4/4 绿 + job 队列/审计/预算实跑），唯 newapi 真实 LLM 响应被【用户侧 Clash TUN 进程级分流】环境阻塞（curl/python 同 body 200，reqwest 全配置变体 500，fake-ip 全劫持 DNS 无法绕过）——用户修 Clash 分流后一键补验：`ENGRAM_DEMO_LLM_KEY/ENGRAM_DEMO_READER_KEY` 就绪后 `cargo test -p engram-core --test wiki_agent_test demo_real_url -- --ignored`（模型 MiniMax-M3）。

## Done

- [x] **破案 + 接口验证 + 工具面盘点**（2026-10-01）：EN-32 断点定性；webReader 实测通过、key 落 credentials `zhipu/web_reader_key`；MCP wiki 工具面 24 action 现状 vs HTTP 差距 12+ 盘点完成。
- [x] **T001 · embed 守卫**（ae069d0）：last-attempt 缺失块 embed_failed + 文档 set_ready（FTS-first 降级）；回归测试 embed_transient_exhaustion_lands_ready_not_orphan。
- [x] **T007 · MCP 工具面全量对齐**（2cd036a）：13 新 action，24→41 工具总数；curation 测试读写双路径绿。
- [x] **T005 · web-reader 接入**（aaae960）：core::wiki_docs::web_reader（MCP streamable HTTP + SSE + 双重 JSON 适配）；未配回落/故障降级事件/真网三测试绿。
- [x] **T009 · 织入流水线退役**（2c22833）：四触发点拆除（MCP ingest action/HTTP 端点/document_add 尾部/write_page auto_ingest）；golden+openapi+mcp_test 同步；service.ingest 与 job 链代码保留（T010 换芯）。
- [x] **T010 · Agent Harness**（ca49dc6 主体 + 6e483c1 加固）：core::wiki_agent（10 工具循环/预算 20 轮 60 调用 10 分钟/全工具 audit 留痕/Purpose::WikiAgent 档位/document_add ready 自动接力）；ingest 同名换芯（喂原料给 harness）；mock 集成 4 场景绿。demo 补验见 Deferred。
- [x] **T006 · 设置页网页读取配置**（15ae465）：GET /wiki/webreader/status + POST test（admin 门）+ Settings「网页读取」tab。
- [x] **T002 · 滞留恢复机制**：reembed MCP action（随 T007）+ K1 姿势入 T008 手册。
- [x] **T003 · 三篇滞留救活验证**：三篇 documents_search 全命中且 embed_failed 全 false（01a0f7e0-a87d-7f22 / 01a0f7e6-03e5-76e3 / 01a0f7e6-03ab-76c1）——生产已自行恢复，无需恢复动作。
- [x] **T004 · EN-31 落档**：检索层/知识层框架+嵌入降级语义进 wiki-engine 文档 v3；EN-31/EN-32 双 resolved。
- [x] **T008 · 维护工作流手册**：engram projects《wiki 维护工作流手册（agent-first）》（01a0f998-78b0）——调用序/harness 触发/预算口径/恢复姿势。
