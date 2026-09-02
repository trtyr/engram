# 合并方案：Knowledge + Wiki → 一个「Wiki」（Obsidian 式知识库）

## 愿景（用户方向 2026-09-02）

合并后的 Wiki = **Obsidian 式的知识库**：本地 Markdown 笔记 + 双向链接（`[[wikilink]]`）+ 知识图谱（graph view）。

一句话：把资料丢进去 → 系统存原料（可 RAG 检索原文）→ AI 自动织成互链页面 → 你在图谱里看知识怎么连在一起。

## 现状盘点（已埋的两块砖）

| 能力 | 现状 | 位置 |
|---|---|---|
| 双向链接 | `[[slug]]` wikilink 已支持（WikilinkText 渲染 + 跳转） | web/src/components/WikiMarkdown.tsx |
| 链接图 | `wiki_links`（from_slug→to_slug，wikilink weight 3.0 / source-overlap weight 4.0） | migration 0007 |
| 图谱 | sigma 力导向（节点=页面，label 已修，degree 定大小，社区发现，theme 联动，32px 网格） | web/src/components/WikiGraph.tsx + GET /wiki/graph |
| 原料 RAG | documents + chunks（分块 + 向量 + FTS，检索原文） | migration 0006 |
| 成品页 | wiki_pages（Markdown，10 种 page_type，版本演进） | migration 0007 |

## 数据模型（原料层 + 成品层共存）

不拆表，两层共存在一个「Wiki」域下：

- **原料层**（原 Knowledge）：`documents`（title/source_uri/mime/raw_path/sha256/status）+ `chunks`（分块 + embedding + tsv）
- **成品层**（原 Wiki）：`wiki_sources`（源）+ `wiki_pages`（Markdown 页面 + frontmatter + version）+ `wiki_links`（双向链接）
- **桥梁**：`documents.sha256` ↔ `wiki_sources.sha256`（都 UNIQUE），自动织时靠 sha256 去重

## API 迁移（/knowledge → /wiki）

| 原端点 | 迁移后 |
|---|---|
| GET/POST /knowledge/documents | GET/POST /wiki/documents |
| POST /knowledge/upload | POST /wiki/upload |
| GET/DELETE /knowledge/documents/{id} | GET/DELETE /wiki/documents/{id} |
| GET /knowledge/documents/{id}/chunks | GET /wiki/documents/{id}/chunks |
| POST /knowledge/documents/{id}/re-embed | POST /wiki/documents/{id}/re-embed |
| POST /knowledge/search | 并入 POST /wiki/search（分层：documents + pages） |

保留 `/knowledge/*` 兼容别名（重定向或双注册）一个过渡期，避免 CLI / skill 立刻断。

## 自动织管线（核心新能力）

上传/摄取文档 → ① 存 documents + chunks（原料，立即可 RAG 检索原文）→ ② 后台队列自动把文档全文喂给 wiki-engine（分析 → 生成页面 → 建 wikilink）→ ③ 图谱出现新节点 + 链接。

- **去重**：sha256 命中已有 wiki_sources → 跳过织（同一资料不重复织）
- **失败隔离**：织失败不影响原料（文档照常可 RAG 检索），页面层降级为空
- **可观察**：织任务落 jobs（kind=wiki_generation），页面显示「织入中」

## 前端融合（一个 Wiki 页）

一个 Wiki 页面，tabs（合并两边）：

- **文档**（原 Knowledge）：上传/URL/列表/阅读 + RAG 检索原文
- **页面**（原 Wiki pages）：AI 织的 Markdown 页 + wikilink 双向跳转
- **图谱**（原 Wiki graph）：Obsidian 式 graph view——力导向 + 节点 label + 拖拽 + 缩放 + 点击跳转 + hover 高亮邻居 + 社区色
- **人审 / 提案 / 目标**（原 Wiki 治理）

删 Knowledge.tsx（或 /knowledge 重定向到 /wiki）。

## 知识图谱（Obsidian graph view 升级点）

现有 WikiGraph 已有基础，合并后按 Obsidian graph view 补齐：

1. hover 节点 → 高亮其邻居 + 淡化其余（聚焦关系）
2. 节点 label 常显（当前 labelRenderedSizeThreshold 3.5，进一步降到全显 + 缩放分级）
3. 点击节点 → 跳转页面（已有）
4. 拖拽节点（复用 EntityGalaxy 的 captor-disable 模式）
5. 图例：页面类型色 + 链接类型（wikilink vs source-overlap）

## 非目标

- 不做 Obsidian 的插件体系 / 全量 Markdown 编辑器（现有 Markdown 渲染 + 阅读足够）
- 不做实时协作 / 多设备同步
- 不动 Memory（用户记忆）域——那是「关于用户」，Wiki 是「世界的知识」
