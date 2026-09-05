# 0006 AI 入口定为 MCP；检测导入与 context pack 降级为约定

**日期**：2026-09-05　**状态**：已拍板

## 背景

用户记忆 MCP 落地后，项目记忆域也并入同一 `/mcp` 端点（15 个 `project_*` 工具，
2026-09-05，含精确寻址读：索引模式 / 行级搜索 / 区间精读）。roadmap Next 里两项
悬而未决的形态问题随之失去前提：

1. 「AI 侧接入：memory.py 项目子命令」——实际上 skill 的 `_project.py` 早已
   全套实现（16 个子命令包 HTTP API），剩的只有「检测本地导入」的扫描约定；
2. 「context pack 接口」——当初动机是 `project_get` 全量倒文档太重，如今
   精确寻址读已让 AI 按需自组上下文。

所有者拍板：**不再投入这两项的代码形态**。

## 决策

### 1. 项目域 AI 入口 = MCP 工具面

- 0005「双入口平等」中的 AI 入口载体由 memory.py 子命令**改为 MCP 工具面**；
- memory.py 现有项目子命令保留不删（无 MCP 环境的备用入口：纯终端/脚本），
  但**不再新增功能、不跟进新特性**（如精确寻址读只在 MCP 层提供）；
- 「检测本地导入」**不做平台功能、不做扫描代码**：AI 用自身文件工具读本地
  markdown → MCP `project_doc_add` 写入云端；「去哪些目录找、如何映射分类」
  是约定文字，落在 MCP instructions / skill 说明，不是代码。

### 2. context pack 协议降级

- 「开工拉上下文」= `project_get`（索引模式）→ `project_doc_search`（定位行号）
  → `project_doc_get`（区间精读），AI 按需自组，不设专用端点/协议；
- 「收工沉淀」= `project_doc_add`/`project_doc_update` + `project_update` 改状态，
  落点约定（写哪个分类、状态怎么改）写入 MCP instructions，不是协议；
- **唯一保留的可选项**：大项目的 LLM 蒸馏「项目简报」（近期文档 → 短摘要，
  走平台 LLM 任务队列）——等文档规模成为真实痛点再议，进 Deferred。

## 后果

- roadmap Next 销掉两项（AI 侧接入 / context pack 接口），剩两项排期项不变；
- open-questions 3、4 转已决（归本决策）；
- MCP instructions 已含开工/收工与寻址读工作流（2026-09-05 落地时写入），
  skill 侧无需改动。
