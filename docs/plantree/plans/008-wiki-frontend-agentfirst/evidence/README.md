# P008 证据索引

## 门禁留档（/tmp，会话级）

| 闸 | 文件 | 结果（HEAD=f8058d3） |
|---|---|---|
| workspace | `/tmp/gate_s_ws_raw.log` | WORKSPACE_EXIT=0；79 ok / 0 FAILED（16:24:28→16:31:26Z） |
| clippy 全仓 | `/tmp/gate_s_clippy_raw.log` | CLIPPY_EXIT=0（16:24:28→16:25:08Z） |
| web 三连 | `/tmp/gate_s_web.log` | TSC_EXIT=0 / LINT 0 error / Tests 89 passed / BUILD_EXIT=0 |
| demo（dist 产物） | `/tmp/gate_s_demo.log` | 新面板全命中（维护 Agent 2/知识缺口/重复页/版本历史/喂给维护 Agent）+退役词 0 |

历次门禁与根因（全部保留，供追溯）：
- gate_l（EXIT=101）：抓出 mcp_wiki_curation_test 两用例调已删的 proposals/proposal_apply；
- gate_n/o/p：提交与门禁并行 → HEAD 错位，作废；
- gate_q（EXIT=0）：修正后通过，但随后又发现 web 侧漏网（ReviewQueue.tsx 孤儿组件+失效测试）；
- gate_r（EXIT=101）：抓出 study_repo_test 夹具未保活（TestPg 随函数返回 drop → 57P01，并行 1/5 挂）；
- **gate_s（HEAD=f8058d3，EXIT=0）：终局，四闸全绿**。

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
