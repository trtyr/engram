# Wiki Gap 分析

对照理论最佳实践（Karpathy 原文 + 两个落地分支）与评论区实战教训，审视当前 `crates/wiki-engine` 实现。核心结论：**已相当完整，踩在「个人规模已验证、团队规模正在填坑」的节点上**；真正值得做的代码改进只有两项，其余是增强项。

## 已补（相对理论最佳实践）

| 能力 | 理论出处 | 实现落点 |
|---|---|---|
| 两阶段 ingest | TencentDB / llm_wiki | `ingest.rs` analyze→generate |
| 矛盾检测（conflicts） | Karpathy 原文 / llm_wiki | analysis `conflicts[]` + `comparison` 页 |
| purpose（方向意图） | llm_wiki 原创 | `purpose.rs`（settings 表） |
| 4-signal 相关性 | llm_wiki | `relevance.rs`（3/4/1.5/1） |
| Graph Insights | llm_wiki | `insights.rs`（4 类） |
| 异步 Review（预定义动作） | llm_wiki | `review.rs`（create_page/deep_research/skip/flag） |
| 级联删除 | llm_wiki / TencentDB | `cascade.rs`（3 路径匹配） |
| lint（5 规则） | Karpathy / TencentDB 自认缺失 | `lint.rs` ✅ **已补** |
| origin=human 保护 | 评论区 huachen-wang 教训 4 | generate 的 human 分支 ✅ **pin 简化版** |
| index/log/overview 系统页 | Karpathy 原文 | `ingest.rs` rebuild_* |
| 结构化检索（FTS + 向量） | 评论区 huachen-wang 教训 3 | `search` + `tsv`/`embedding` |

## 未补（gap）

### G1 并发去重硬机制缺失（中）

评论区 huachen-wang 教训 1：并发 ingest 读到同一份 index 快照 → 各自决定建同名页面 → 近重复（实测 38% 近重复）。

当前只有 `sha256` 幂等（同内容跳过）+ prompt 软约束「不要建近重复页」。缺「index 占位符（planned 状态）+ 原子条件写入」硬机制。generate 里 slug 冲突靠 `ON CONFLICT` + origin 分支兜底，没有 planned 状态。

- 影响：单用户单 Agent 够用；团队多 Agent 并发 ingest 会撞坑。
- 备注：本项目定位单用户平台，此 gap 优先级可能低。

### G2 pin 存活机制不完整（中）

huachen-wang 教训 4：人类修正要「记录意图（claim）+ 锚定小节 + 重编译后核对」，而非存 diff。

当前 `origin=human` 是**整页级**保护：LLM 不覆盖人写的页，只发提案。比 pin 粗粒度但简单可靠。缺口：没有「意图记录 + 小节级锚定 + 重编译后核对」，提案合入后也无意图留痕。

### G3 Read Sources Only 缺失（低-中）

llm_wiki 的「安全阀」：可切换「只从原始材料回答，不做 wiki 综合」，对抗「幻觉固化进 wiki」。当前检索只有 FTS + 向量，无「仅原文」模式。

### G4 查询侧按类型路由缺失（低）

评论区 tonydzi 469 轮对比：主题问题（「关于 X 知道什么」）wiki 侧更好 71%，实体查找（「这个人是谁」）只有 10%。当前 `search` 统一 FTS，无「主题 vs 实体」分流。

### G5 log 只记 ingest（低）

TencentDB 案例缺口 4：log 应记 ingest + query + lint。当前 `update_index_and_log` 只追加 ingest 记录；query/lint 未进 log.md（但有 `job_events` 全量兜底）。

### G6 copied state 教训（写作规范层）

评论区 WadeGIMPBC：文档不记会变的值（SHA/行数/日期），只记「契约的形状」。这是 conventions 层面，非代码 gap。

## 结论

- 实现覆盖了理论谱系的绝大部分设计，且补上了 TencentDB 自认缺失的 lint。
- 真正值得做的代码改进：**G1 并发去重**、**G2 pin 存活**（若未来有多 Agent / 并发 ingest 需求）。
- G3~G6 属增强项，优先级低。
