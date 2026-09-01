# 圈子（/circle）分离审计报告

> 2026-09-01 四维分离审计：①功能现状 ②前端现状 ③功能缺口 ④开放 API。
> 纯审计落档，零代码改动。活体基线：生产栈 :19180（16 实体 / 13 共现边 / 19 迁移 / 69 路径）。

## 核心洞察（先读这条）

圈子的**图**是它的灵魂，但图的价值由**数据密度**决定：

- 密度定大小 = `5 + min(atom_count × 1.1, 9)`；共现边粗细 = `weight`。
- 当前生产库 **16 个实体全部 atom_count=1、13 条共现边全部 weight=1**（测试方重建后每个实体只挂 1 条原子）。

所以在数据成熟前，圈子图退化成一张**均匀星形点名册**——「我」居中，16 个等大节点围一圈，边全是细线，看不出谁重要、谁和谁抱团。这不是 bug，是「图的能力 > 当前数据密度」的结构性落差。审计里所有「图观感弱」的问题，根子都在这里；而「实体关系升级」这类大项，要等密度上来才真正有意义。

第二个结构性发现：**详情页与图谱割裂**——`EntityDetail = {entity, atoms, scenarios}`，没有「相关实体/共现邻居」。用户点进一个实体，看不到「它和谁相关」，必须退回图谱看边。这是当前最直接的体验断裂，且修起来便宜（后端 get_entity 加一个邻居查询）。

---

## 一、功能现状

圈子 = 记忆模型「一坐标系」的独立页（/circle，与侧栏「代码图谱」对称），按 WHO/WHAT 浏览记忆：实体（person/project/topic/group/place 五类）。

| 功能 | 现状行为 |
|---|---|
| 实体列表（左栏） | N 实体计数 + 搜索（name/summary 子串，**客户端 contains**）+ 类型过滤 chips + 密度降序（后端默认 `atom_count DESC`，**无显式排序控件**） |
| 新建实体 | 名 + 类型下拉 + 摘要，同名同类去重（后端 400/活体唯一名） |
| 关系图谱（右栏） | sigma 力导向（forceAtlas2 2.5s 自组织）；「我」fixed 居中；类型着色 + 密度定大小；共现边粗细随 weight；**节点可拖拽**（captor-disable 修复）；点节点进详情；图例三定义 + 类型色 chips |
| 实体详情 | 摘要（点击编辑 + 已钉住标记）、原子时间线（状态定宽列）、相关场景 chips、挂原子（最近 100 条池）、摘除、合并进其他实体（confirm）、删除（confirm，原子保留）、返回图谱 |
| 编辑分权 | PATCH 字段白名单：AI 禁改 content/kind/confidence；entity summary 手编 stamp `manually_edited=true`，consolidate 绕开 |
| forget 级联 | DELETE `?forget=true` 归档 active 原子 + 删实体（返回 `{archived:N}`） |

## 二、前端现状

**组件结构**（3 文件）：

| 文件 | 行数 | 职责 |
|---|---|---|
| `Circle.tsx` | 32 | 薄壳：PageHeader + Galaxy + `?entity=` 深链 + 跨页跳转（persona/atoms） |
| `Galaxy.tsx` | 520 | 主体：列表 + 图 + 详情 + CreateEntityForm/AttachForm/MergeForm 六子组件 |
| `EntityGalaxy.tsx` | 147 | lazy sigma 图：forceAtlas2 自组织 + 拖拽 + 主题联动 |

**六维审计**：

- **交互**：搜索/过滤/新建/挂摘/合并/删除/编辑摘要齐全；双栏独立滚；图例三定义（`圆点/连线/「我」`）；详情操作按钮固定右上（不随摘要换行漂移）。
- **视觉**：Engram 墨白 + 五类实体中明度色（`person #e6772e / project #3b82f6 / topic #10b981 / group #ec4899 / place #14b8a6`），双主题通用。
- **响应式**：`h-[calc(100vh-13rem)]` 双栏，`lg:flex-row` 断点，移动端纵向堆叠。
- **性能**：sigma 157kB **独立 lazy chunk**（不进 initial 282kB）；graph 一次 fetch 供列表+图复用；详情按需 fetch；forceAtlas2 2.5s 动画后定格。
- **可访问性**：列表 `button + aria-current` 良好；**图是 canvas，无键盘可访问性、无 aria 语义、拖拽无键盘替代**（缺口）。
- **主题**：`useThemeTick` 联动图重绘，`ENTITY_KIND_COLOR` 中明度双主题安全。

## 三、开放 API（活体实测）

entities 面 **5 路径 / 9 方法注册**（openapi-dump + curl 实测，全部 `require_memory` scope）：

| 方法 | 路径 | 返回 | 说明 |
|---|---|---|---|
| GET | /memory/entities | `EntityDto[]` | `?kind=` 过滤；`atom_count DESC` 排序 |
| POST | /memory/entities | `EntityDto` | `{name,kind,summary}`；同名同类去重 |
| GET | /memory/entities/graph | `{nodes, edges:{a,b,weight}[]}` | 共现边 = 同原子双关联 count |
| GET | /memory/entities/{id} | `EntityDetail{entity,atoms≤200,scenarios≤50}` | 无邻居字段 |
| PATCH | /memory/entities/{id} | `EntityDto` | `{name?,summary?}` |
| DELETE | /memory/entities/{id} | 204 或 `{archived:N}` | `?forget=true` 级联归档 |
| POST/DELETE | /memory/entities/{id}/atoms/{atom_id} | 204 | 挂/摘原子，幂等 |
| POST | /memory/entities/{id}/merge | `{moved:N}` | `{into}`；loser 置 merged_into |

api-schema.ts（3786 行）与后端 OpenAPI 零漂移（EntityDto/EntityGraph/EntityDetail/GraphEdge + 8 operations 全在）。

**API 面缺口**：无独立实体检索端点（`search_entities` 只内嵌在 search/context_pack，未暴露 route）；无「相关实体/邻居」端点；无实体导出端点（嵌 /memory/export）；无批量操作端点；无实体版本/历史端点（对比 atom 有 `/revisions`）。

## 四、功能缺口（含全新能力候选）

1. **实体关系是「共现」不是「关系」**——边只有 weight（同原子共现次数），无类型/方向/语义。表达不了「张三 ∈ 后端组」「上海 = 长亭科技所在地」。（e2174fda9b28 MVP 取舍的显式遗留；本次用户松口「可引全新能力」，需重拍板）
2. **圈子检索缺失**——前端搜索只是 substring contains（客户端），后端无独立实体检索端点，无向量/语义检索。
3. **无时间轴视图**——详情有原子时间线，但无「按时间浏览实体/记忆」的独立视图。
4. **无批量操作**——merge 单对单、删除单实体、挂摘单原子。
5. **详情与图谱割裂**——详情无「相关实体/共现邻居」，退回图谱才看得到关系。
6. **无实体级去重/合并建议**——arbitrate 管原子不管实体；「周杰伦」vs「Jay Chou」无自动提示。
7. **无实体导出**——记忆域 export 含 entities，圈子无独立实体导出。
8. **图可读性**——低密度下均匀星形；无 zoom/pan reset 控件；无社区聚类；节点多时标签重叠。
9. **摘要生成门槛**——consolidate 只给 `atom_count>=3` 的实体生成画像，稀疏实体长期「尚无画像摘要」。
10. **无实体历史/版本**——summary 编辑无版本链（atom 有 revisions）。

## 五、P1/P2/P3 分级强化清单

> 分级口径：P1 = 信任/正确性断裂，便宜且直接；P2 = 能力补齐，让圈子「变强大」；P3 = 大项，需拍板（含全新能力）。

### P1（便宜、直接修，建议实施轮优先）

| 项 | 现状 | 问题 | 建议 | 预估 |
|---|---|---|---|---|
| 详情补「相关实体」 | get_entity 无邻居 | 详情看不到关系，退回图谱看边 | 后端 get_entity 加共现邻居查询 + 前端详情「相关实体」chips | 小（后端 1 查询 + 前端 1 区块） |
| 实体级去重/合并建议 | 无 | 同名同义实体靠人工发现 | 后端 list 时按 name 归一化聚类提示候选 merge | 中（需定归一化规则） |
| 图可读性：zoom/pan reset | 无控件 | 拖/缩丢失后回不来 | 前端加 reset 按钮 + 适配缩放 | 小 |
| 图可访问性 | canvas 无 aria/键盘 | 图对键盘/读屏不可达，WCAG 基础缺口 | canvas 加 aria-label + 键盘焦点替代（节点遍历） | 中 |

### P2（能力补齐）

| 项 | 现状 | 问题 | 建议 | 预估 |
|---|---|---|---|---|
| 圈子语义检索 | 前端 substring，后端无独立端点 | 记不住名字就找不到实体 | 后端独立 `GET /memory/entities/search?q=`（复用 search_entities token 打分）+ 前端语义搜索框 | 中 |
| 图可读性：社区聚类 | 无 | 实体多时一团 | 后端 graph 返回社区标签（louvain）或前端按 kind 分区 | 中 |
| 低密度信息呈现 | 均匀星形 | 数据成熟前图无信息量 | 空态/低密度态提示 + 列表为主、图为辅的降级 | 小 |
| 实体摘要补生成 | 门槛 atom_count>=3 | 稀疏实体长期无画像 | consolidate 门槛降或按需触发单实体画像 | 中 |
| 实体历史/版本 | 无 | summary 编辑无版本链、无实体版本端点（对比 atom 有 /revisions） | 实体摘要版本链 + 历史抽屉，复用 atom revisions 模式 | 中 |

### P3（大项，需拍板）

| 项 | 现状 | 问题 | 建议 |
|---|---|---|---|
| 实体关系升级 | 共现边 | 共现无法表达成员/地点等语义关系——「张三∈后端组」「上海=长亭科技所在地」都说不出来 | 类型化关系表（关系类型+方向）或折中「共现+关系摘要」；需拍板：建表/关系枚举/谁写（蒸馏 vs 手动） |
| 时间轴视图 | 无 | 无法按时间浏览实体/记忆，时间维度完全缺失 | 实体时间线或全局记忆时间轴（需拍板视图形态） |
| 批量操作 | 无 | merge/delete/归档只能单对象操作，批量管理低效 | 批量 merge/delete/归档；破坏性批量需 erase 分权 + 二次确认 |
| 实体导出 | 嵌 /memory/export | 圈子无独立导出，实体数据主权出口不清晰 | 圈子独立实体导出（csv/json）；需拍板形态与隐私口径 |

---

## 附：审计方法

- 活体基线：`openapi-dump`（69 路径 / 86 注册）+ curl 探针（entities 列表 16 条、graph 13 边全 weight=1）+ 生产栈 :19180。
- 源码：`Circle.tsx`（32）/`Galaxy.tsx`（520）/`EntityGalaxy.tsx`（147）/`memory.rs`（entity_graph/list_entities/get_entity）。
- 前端类型：`api.ts` EntityNode/EntityGraph/EntityDetail + `api-schema.ts` 3786 行（零漂移）。
- bundle：initial 282kB / sigma 157kB lazy（411f91485460 基线）。
