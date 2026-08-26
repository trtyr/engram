# 链接图、相关性与洞察

Wiki 把「页面之间的关联」显式建模成一张加权有向图，并在此之上做社区发现与洞察推荐。涉及 `markup.rs`（wikilink 解析）、`relevance.rs`（4 信号）、`community.rs`（Louvain）、`insights.rs`（洞察）。

## 链接图（GraphDto）

节点 = 页面（`wiki_pages`），边 = `wiki_links`。`GET /wiki/graph` 返回：

```json
{
  "nodes": [{"slug", "title", "page_type", "community"}],
  "edges": [{"from_slug", "to_slug", "weight"}],
  "communities": [{"id", "size", "cohesion"}]
}
```

`community` 是 Louvain 社区 id（前端着色切换用）。

## wikilink 解析（markup.rs）

`extract_wikilinks(content)` 扫描 `[[...]]`：

- `[[slug]]` → slug
- `[[slug|显示名]]` → 取 `|` 前的 slug
- 去重保序；只保留 `is_valid_slug` 通过的 target

边生成（`ingest.rs::rebuild_links`）：对每页提取 wikilink，删旧出边，插入 `wiki_links`（weight 3.0，`ON CONFLICT DO NOTHING`）。

## 4 信号相关性模型（relevance.rs）

对齐 llm_wiki，为每条边计算相关性权重：

| 信号 | 权重常量 | 含义 |
|---|---|---|
| 直接链接 | `W_DIRECT = 3.0` | 有 wikilink 边 |
| 源重叠 | `W_SOURCE_OVERLAP = 4.0` | 两页 `frontmatter.sources[]` 有交集 |
| Adamic-Adar | `W_ADAMIC_ADAR = 1.5` | 共同邻居的度倒数之和（封顶 1.0） |
| 类型亲和 | `W_TYPE_AFFINITY = 1.0` | 两页 page_type 相同 |

`relevance_score(direct_linked, shared_sources, adamic_adar, same_type)` 是纯函数（单测覆盖），得分 = 各命中信号权重之和。理论上限 3 + 4 + 1.5×1 + 1 = 9.5。

### Adamic-Adar

`adamic_adar(neighbors_a, neighbors_b, degree)`：对两页的共同邻居求和 `1 / ln(degree)`（度取对数、封底 1）。捕获「两个页面通过哪些枢纽节点相连」的强弱。

### 权重重算：rebuild_weights

每次 ingest 后有新增/更新时全量重算（O(n²) 邻居对，百页级可接受）：

1. **直接链重算**：对每条已有 `wiki_links` 边，用 4 信号重算 weight（最低 0.1）。
2. **源重叠补边**：遍历所有页面对，若 `shared_sources > 0` 且**无直接链**，补两条有向边（weight = 4.0 + 类型亲和），把「同源但没互链」的页面连起来。

补边是无向语义（两条有向边都补），与 wikilink 边形态一致。

## Louvain 社区发现（community.rs）

Rust 实现的**简化单层 Louvain**（无向加权图上贪心节点迁移）：

- 初始每节点独立社区，反复把节点迁移到模块度增益最大的邻社区，直到无提升（EPS=1e-12 防震荡，最多 n 轮）。
- 返回 `slug → community id`（重编号为 0..k）。

`community_cohesion(nodes, edges, communities)`：社区凝聚度 = 社区内实际边权重 / 可能边数（无向对 `n(n-1)/2`）。低凝聚（<0.15）会被 insights 标记为稀疏社区。

图规模（百页级）用简化实现足够；注释明言是「近似」而非完整多层 Louvain。

## 图洞察（insights.rs）

`POST /wiki/insights` 在链接图上算出 4 类洞察，均可 dismiss（持久化到 `wiki_insight_dismissals`，稳定键 `类型:slug`）：

| kind | 判定 | 含义 | 建议动作 |
|---|---|---|---|
| `isolated_page` | 度数 ≤ 1 | 孤立页面 | 补互链或合并到相关主题页 |
| `sparse_community` | 社区 ≥3 页 且 凝聚度 <0.15 | 稀疏知识区 | 补综述/对比页把它们连起来 |
| `bridge_node` | 连接 ≥3 个社区 | 桥节点 | 更新它影响面最大，保持精炼准确 |
| `surprising_connection` | 跨社区 + 跨类型 + 强边（权重 > max×0.5） | 意外连接 | 可能是新洞察，也可能需修正 |

每项洞察携带 `slugs`（前端高亮）与 `search_queries`（深研检索词）。

dismiss 相关 API：`POST /wiki/insights/dismiss`（单个）、`POST /wiki/insights/reset`（全部重置）。

## 相关端点

- `GET /wiki/graph` — 图数据（节点/边/社区）
- `POST /wiki/insights` — 洞察报告
- `POST /wiki/insights/dismiss` — dismiss 单个洞察
- `POST /wiki/insights/reset` — 重置全部 dismiss
