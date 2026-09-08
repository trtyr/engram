# Engram MCP 工具面（AI 客户端权威文档）

> MCP 端点：`POST /mcp`（Streamable HTTP，JSON-RPC；Bearer 认证用 amk_ key）。
> 设计：渐进式发现——AI 常驻只见 **7 个入口工具**，域内操作按需发现，三层同源：
> L0 每个工具描述内嵌「操作目录」→ L1 `{"action":"help"}` 取全域参数手册 →
> L2 未知操作/坏参数的报错附合法清单。

## 调用形态

```json
{"name": "wiki", "arguments": {"action": "write_page", "slug": "demo", "title": "示例", "content": "..."}}
```

- `action` 必填；其余参数平铺在顶层（与 help 返回的 schema 一致）；
- 工具级 scope 检查 + 操作级停用开关（`域.action`）由服务端执行；
- **写操作不回显正文**：统一返回 `{元数据..., content_omitted: true, content_chars: N}`（HTTP API 不受影响）。

## 七个入口工具

| 工具 | scope | 操作数 | 一句话 |
|---|---|---|---|
| `memory` | memory | 10 | 用户记忆：L0-L3 四层 + 实体，写会话/检索/遗忘 |
| `projects` | project | 16 | 项目记忆：目标/位置/分类文档树/行级补丁 |
| `skills` | skills | 10 | 可复用指令包：SKILL.md + 附属文件 + 版本回滚 |
| `wiki` | wiki | 15 | 多库知识库：页面/织入/检索/版本/原料 |
| `todos` | todos | 6 | 快速待办 |
| `codegraph` | codegraph | 6 | 代码图谱查询 |
| `search_all` | 任一域 scope 可见 | — | 跨域全局检索（memory/wiki/skills/todos/projects 并发各回 top-k） |

tools/list 按 key 的 scope 过滤：只有部分域 scope 的 key 看不到无权域工具；
`search_all` 只要持有任一可检索域 scope 即可见（域内结果再按 scope 分域执行）。

## memory（10 操作）

| action | 说明 |
|---|---|
| `context` | 装载上下文包（画像+场景+原子+实体；`include_evidence` 默认不携带溯源 ID） |
| `search` | 跨 L1/L2/L3/实体混合检索（`from/to` 时间窗、`include_evidence` 开关） |
| `remember` | 一句话记忆（等价单轮 write_session + auto 蒸馏） |
| `write_session` | 写会话（主写入口；蒸馏自动抽取；`distill`: auto/manual/off） |
| `append_session` | 未蒸馏会话追加轮次 |
| `list_sessions` / `get_session` | 会话列表（keyset 分页）/ 逐轮原文 |
| `list_atoms` | 原子浏览（**默认只回 active**；`status:"all"` 看全量） |
| `entities` | 实体检索（人物/项目/主题/群组/地点） |
| `forget` | `void` 作废（级联归档产物）/ `erase` 物理删除（需 erase scope）/ `restore` 撤销作废 |

分权：原子/画像/实体的直接语义改写是用户（Web）专属；AI 纠错写纠正会话，蒸馏自动生成取代链。
凭据类（密码/密钥）蒸馏主动跳过；敏感会话（sensitive=true）产物默认不进检索与上下文。

## projects（16 操作）

`types` `list` `get`（索引模式，`include_content` 才带正文）`create` `update`（改名/状态/描述/分类）
`delete` `batch_delete` `location_add/update/delete`（多主机登记）
`doc_add` `doc_get`（全文或行区间，恒带行号）`doc_search`（grep 式按行）
**`doc_patch`**（行级 replace/insert/delete——改长文档不必取全文重发）`doc_update` `doc_delete`

## skills（10 操作）

`list` `get` `file_get` `file_put`（附属文件；SKILL.md 本体走 update）
`create` `update`（语义变更自动留快照）**`versions`**（快照列表）**`restore`**（回滚，本身也留快照）
`delete`（仅限用户明确要求）`import`（SKILL.md frontmatter 容错解析）
寻址：slug 优先，技能名精确匹配兜底；附属文件路径禁 `..`/绝对路径/盘符冒号（NTFS ADS）。

## wiki（15 操作，多库）

全部操作接受可选 `library` 参数（库 slug，缺省 main）。

| action | 说明 |
|---|---|
| `libraries` | 列出全部库（slug/名称/页面数/原料数；建库/删库走 Web） |
| `write_page` | 写/覆盖页面（旧文自动留版本快照；`folder` 目录树可选） |
| `get_page` / `list_pages` | 读全文（title 也寻址）/ 浏览（page_type 过滤 + keyset 分页） |
| `search` | FTS+向量融合；命中带片段 + content_chars，全文按需 get_page |
| `ingest` | 整篇织入（异步 LLM 流水线；sha 库内去重；三态 ready/in_flight/enqueued；产物落同库） |
| `archive_query` | 问答存档为 queries 页（幂等；同标题跳过） |
| `versions` / `version_content` / `restore_version` | 版本史（每页 50 版）/ 预览正文 / 回滚（删除页可重建） |
| `sources` / `delete_source` | 织入原料列表 / 级联删除（页面+任务+源） |
| `graph` / `lint` | 链接图（Louvain 社区）/ 体检（死链/孤页/缺源/相似目录） |
| `delete_page` | 删页（双向 wikilink 清理；最后状态留快照可重建） |

## todos（6 操作）

`add` `list`（status/priority/tag/q 过滤 + keyset 分页）`get` `done`（幂等）`update` `delete`
批量操作（批量恢复/批量擦除）目前经 HTTP `/memory/...`——不对，待办批量在 Web 会话页与
HTTP API；todos MCP 面保持六操作。

## codegraph（6 操作）

`list` `register`（本地路径按**服务端**文件系统校验；跨机器用 git URL）
`index` / `sync`（异步 job；纯 README 仓库 0 符号为正常行为）
`query`（kind = search / **explore（默认符号大纲，include_source=true 才带源码）** /
node / callers / callees / impact）`delete`

## search_all（跨域）

`{ "query": "...", "max_per_domain": 3 }` → memory / wiki / skills / todos / projects 五域并发，
各回 top-k 摘要；只检索 key 有 scope 的域；已知域的精确检索请用单域工具。

## 分权规则（SERVER_INSTRUCTIONS 摘录）

- 用户记忆语义内容的直接改写（原子/画像/实体档案）是用户专属；AI 写入只有「会话」通道；
- 敏感对话写入时置 `sensitive=true`（产物默认不进检索与上下文）；
- 破坏性操作（delete/forget 类，目录有【破坏性】标注）只对用户明确请求使用；
- skills delete 仅限用户明确要求——过时用 update，改坏用 restore。

## 管理与可观测

- Web「MCP」页：服务总开关、域工具/单操作停用开关、与 AI 实际所见同源的工具面预览；
- 停用的操作从目录/手册隐身且调用直接拒绝；
- 每个 wiki 织入/文档摄取 job 可 `GET /jobs/{id}` 查进度。
