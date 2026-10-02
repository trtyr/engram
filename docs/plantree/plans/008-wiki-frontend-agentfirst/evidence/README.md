# P008 证据索引

## 门禁留档（/tmp，会话级）

| 闸 | 文件 | 结果（HEAD=352bc81） |
|---|---|---|
| workspace | `/tmp/gate_q_ws_raw.log` | WORKSPACE_EXIT=0；79 ok / 0 FAILED（15:23:09→15:51:41Z） |
| clippy 全仓 | `/tmp/gate_q_clippy_raw.log` | CLIPPY_EXIT=0（15:23:09→15:27:12Z） |
| web 三连 | `/tmp/gate_q_web.log` | TSC_EXIT=0 / LINT 0 error / BUILD_EXIT=0 |
| demo（dist 产物） | `/tmp/gate_q_demo.log` | 新面板全命中（维护 Agent 2/知识缺口/重复页/版本历史/喂给维护 Agent）+退役词 0 |

注：gate_l（首轮）被门禁自身抓出 mcp_wiki_curation_test 漏网（EXIT=101）；
gate_n/o/p 因提交与门禁并行导致 HEAD 错位，作废；**以 gate_q 为准**。

## 审计历次驳回与修正

| 轮次 | 驳回要点 | 修正 |
|---|---|---|
| 1 | roadmap Next 与 Done 自相矛盾（终态未清 Next） | b5a60c6 清 In Progress/Next |
| 1 | lint_deep.rs 等注释仍称「写入人审队列」（doc lies） | b5a60c6 三处注释改「随 job report 返回」 |
| 1 | 未指出但自查发现：ingest/generate.rs 仍 INSERT wiki_review_items（功能残留） | b5a60c6 改结构化 warn 日志 |
| 1 | 同上：MCP 组名「织入」未对齐 | b5a60c6 →「原料」 |

## 关键 commit

- 8b97b26 T001+T002 人审机制整体移除（后端+前端+测试）
- f118992 T003 Agent 维护流面板 + HTTP /wiki/ingest
- 7522406 T004 版本管理 UI + HTTP 3 端点
- b27d235 T005 知识缺口/重复页面板
- 5ad386b T006 退役词全清
- 28e0676 T007 todos 主从详情 + 漏网测试清理
- b5a60c6 审计修正（plan tree 终态 + stale 注释 + 功能残留）
