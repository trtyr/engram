# P017 · maintain_project Agent（项目文档定期整理，每项目独立、可并发）

> 2026-10-06 立项。用户原话：每个项目一个维护 Agent，A/B/C/D 四项目并发四个；
> 把文档整理捋一捋；给 Agent 一个工具，让它把发现的问题记录在文档里，方便之后维护。
> 对齐 maintain_memory（P015）/ maintain_wiki（P016）同哲学：定期巡逻的 Agent，
> 人不看运维面板，只看产出报告。

## 形态

### 1. 节律与并发
- 全局节律桶 `rhythm_maintain_project`（每日，rhythm-maintain-project-YYYYMMDD 幂等键），
  桶 handler 扫全部项目 → 每项目投递一个 `maintain_project` 任务（payload 带 project_id）
- 并发天然成立：runner worker 池并发消费各项目的任务
- 单飞：per-project 守卫（`SELECT count(*) FROM jobs WHERE kind='maintain_project'
  AND status='running' AND payload->>'project_id'=$1`）——同项目不重入，跨项目不受限

### 2. Agent 循环（对齐 patrol_agent JSON 协议模式）
- maintenance system prompt：项目文档整理官
- 工具面（JSON 协议 {tool,args}，步数闸门 ~12）：
  - `list_docs {}` → 项目文档树（id/category/folder/title/字数）
  - `get_doc {doc_id}` → 文档全文（截断）
  - `note_issue {doc_id, issue, suggestion}` → 把发现的问题作为「维护注记」写进该文档
    （doc_patch 追加 `## 维护注记` 段，带日期与 Agent 署名——留痕在文档里，人审时可见）
  - `finish {summary, issues_noted}` → 收尾纪要
- 预算闸门：MAX_STEPS 超限优雅收尾；Agent 失败降级 warn 不阻塞巡逻

### 3. 报告
- 每项目每次巡逻一份 Markdown 报告（对齐 P016 巡逻报告）：整理了什么/发现什么问题/
  写了哪些注记——落 job progress，前端项目详情页后续可挂历史列表

### 4. API 面
- POST /projects/{id}/maintain（手动触发，per-project 单飞守卫）
- GET /projects/{id}/maintain/list + /{job_id}（历史列表 + Markdown 详情，巡逻同款）

## Done

- （未开始）
