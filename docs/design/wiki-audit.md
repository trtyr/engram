# Wiki 域深度审计（对照 llm_wiki）

日期：2026-09-02
方法：源码逐文件读 + 调用链追踪 + 测试覆盖盘点 + 前端交互核查。

## 一句话核心洞察

**后端是满配的、真实工作的（不是名存实亡）**——两步思维链摄入、purpose/index/log/overview 真实读写、四信号图谱、Louvain、洞察、级联删除全链路健在且带测试。**真正的缺口全在前端体验层 + 一条「从知识库织入 Wiki」的接线**：后端 `POST /wiki/ingest` 早已支持 `document_id`（从 knowledge 文档织入），但前端没有任何入口，也没有上传后自动触发——「自动织」管线的最后一米没接上。

## 后端能力现状（真实工作，非名存实亡）

| 能力 | 证据 | 位置 |
|---|---|---|
| 两步思维链摄入 | analyze_job → 链式入队 generate_job，purpose+index 真实注入 user prompt | ingest.rs:181/263 |
| purpose 注入 | analyze + generate 都读 `purpose_context`；LLM 可建议更新 purpose（入人审队列） | ingest.rs:203/295 |
| index.md 维护 | `rebuild_index_page` ingest 后调用 + cascade 后复用 | ingest.rs:629 |
| log.md 维护 | append 一行，W7 截最近 500 行 | ingest.rs:587-626 |
| overview.md | `rebuild_overview_page` 每次 ingest 重生成 | ingest.rs:650 |
| 四信号权重 | relevance.rs 完全对齐 llm_wiki（×3/×4/×1.5/×1），ingest 后 rebuild_weights | relevance.rs |
| SHA256 去重 + 断链恢复 | enqueue_ingest sha 命中跳过；W1 状态感知重入队（analyze 成功 generate 失败 → 取 analysis 重发） | ingest.rs:30-178 |
| Louvain + 内聚度 | louvain_communities + community_cohesion + sparse 标记（与 insights 同口径） | community.rs + service.rs:186 |
| 图谱洞察四种 | surprising_connection / isolated_page / sparse_community / bridge_node | insights.rs |
| 级联删除三重匹配 | sources[] 主路径 / 摘要页整删 / 共享页摘源 + W5 清理悬空边 | cascade.rs |
| 异步审核 | analyze 时 review flag 落库 + purpose 建议入队，不阻塞 ingest | ingest.rs:220-250 |
| lint | dead links + orphans，带测试 | lint.rs + wiki_test:287 |
| **knowledge 织入 Wiki（后端）** | `POST /wiki/ingest {document_id}` → `ingest_knowledge_document` → 解析 → 两步摄入 | wiki_api.rs:52 + service.rs:114 |

测试覆盖：wiki_test.rs 9 个（两步无重复 / human 提案 / lint / W1 / W2 / W3 / W4 / W6 / 社区稀疏）+ cascade_test.rs 2 个（级联 / W5）+ relevance.rs 内联 2 个。核心链路全有测试，无「函数存在但无验证」的空壳。

## 前端图谱体验现状（vs llm_wiki）

已有：社区/type 双模式着色、节点度数定大小、label 渲染（threshold 3.5）、点击跳转、主题联动、社区图例 + 稀疏警告、洞察节点高亮。

缺失（对照 llm_wiki graph view）：

| llm_wiki 体验 | 我们现状 |
|---|---|
| hover 邻居高亮（邻居可见、非邻居变暗、边显示关联分） | ❌ 只有 enterNode 改 cursor |
| 拖拽节点 | ❌ 无（EntityGalaxy 有 captor-disable，WikiGraph 没有） |
| 缩放控件（放大/缩小/适应屏幕） | ❌ 只有滚轮 |
| 边粗细/颜色按权重（绿=强灰=弱） | ❌ edge size 固定 1，weight 字段未用 |
| 位置缓存（防布局跳动） | ❌ 节点 x/y 用 Math.random，每次刷新跳 |

## P1/P2/P3 修复清单

| 级别 | 现状 | 问题 | 建议 |
|---|---|---|---|
| P1 | `POST /wiki/ingest` 已支持 document_id，但前端 Knowledge.tsx 无入口 | 「自动织」管线最后一米没接上：上传知识库文档后无法织入 Wiki | 合并方案落地：上传文档后自动触发织入（或「织入 Wiki」按钮），页面显示织入中 |
| P1 | WikiGraph 无 hover 邻居高亮 | 图谱看不出节点之间的关系（Obsidian graph view 核心体验缺失） | 复用 EntityGalaxy 的 captor 交互，加 enterNode → 高亮邻居 + 淡化其余 |
| P2 | WikiGraph 无拖拽 | 无法手动整理布局 | 复用 EntityGalaxy 的 getMouseCaptor().enabled=false 拖拽模式 |
| P2 | 无缩放控件 | 大图无法快速聚焦/复位 | 加放大/缩小/适应屏幕按钮（animatedReset） |
| P2 | edge size 固定 1，weight 未用 | 四信号权重算出来了但图谱不展示强弱 | 边粗细/颜色按 weight 编码（绿=强灰=弱，对齐 llm_wiki） |
| P2 | 节点 x/y Math.random | 每次刷新布局跳（llm_wiki 有位置缓存） | 位置缓存（localStorage 或后端存节点坐标） |
| P3 | 低密度数据下 degree 全 0 | 图谱退化为均匀点阵（size 全 5，label 可能全隐藏） | 低密度降级提示 + 初始布局（按 page_type 扇区） |
| P3 | 文档解析仅 5 格式 | pptx/xlsx/图片无法摄入 | 扩 parsing crate 格式（对齐 llm_wiki 多格式） |
| P3 | 无多模态图片摄入 | PDF 内嵌图片信息丢失 | 视觉模型描述（可选，成本高，Deferred） |
