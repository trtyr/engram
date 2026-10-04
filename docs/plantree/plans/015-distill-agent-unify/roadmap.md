# P015 roadmap

## Done

- **抽取 v9/v9.1（886b5a0/a4dc0e5/041a667）**：准入合一（段级 worth_memorizing/reason 回执）+
  think-off 适配（MiniMax thinking.type=disabled，延迟 -88% token -93%）+ few-shot 判例 +
  ChatRequest extras 透传通道；效果 demo 4 样本全对（陷阱全拒）
- **落库切片（de0d456 本地验证）**：抽取 → conf<0.55 过滤 → 批量向量化（Qwen3-Embedding-8B
  1024 维）→ INSERT → 回读确认
- **正式入库管道（6970dc5）**：extract persist 直落 active + conf<0.55 丢弃 + 直接链 organize；
  在线仲裁整体退役（arbitrate.rs/任务类型/WORKFLOW_KINDS/P_ARBITRATE，-981 行）
- **离线整理 Agent（669874d）**：maintain_memory 七工具 JSON 循环（recent/search/merge/archive/
  persona_doc_read/edit/finish，max_steps=30）+ 迁移 0070 画像活文档（persona_doc 单行+history
  版本链，增量编辑禁推倒重写）+ atom_merge 借 consolidate 语义 + 节律切换（rhythm_consolidate
  每日桶改投整理）+ POST /memory/maintain 手动触发 + GET /memory/persona-doc 只读
- **FTS 全面退役（2026-10-04 拍板「全部退役」，本切片）**：
  - 迁移 0071：drop atoms.tsv / scenarios.tsv 列与 GIN 索引（不可逆；生产 pre-agentic 备份兜底）
  - hybrid.rs 重写纯向量：接口签名保持、内部单路近邻；RRF/FTS 腿删；
    噪声防线（绝对天花板+相对间隔）保留
  - MemoryService.search/context_pack 加 query_vec 参数（调用方自带向量通道，None=内部 embed；
    api/mcp/unified 调用点传 None）
  - 整理 Agent atoms_search 向量化（查询文本现场向量化→近邻）
  - persist 写侧 to_tsvector 移除；transfer 导入去 tsv
  - wiki 域 tsv 不受影响（纯 RAG 知识库独立检索策略，非本 plan 范围）
  - 测试对齐：正交向量助手（vec1024 同 i 命中/异 i 不可达）、MockLlm 伪向量 seed 对齐
    （unified_rerank 输入序前提更新为纯向量序）

## In Progress

（无——待收官门禁：api/mcp 测试套 + fmt/clippy 全绿后提交推送）

## Next

- 生产部署拍板（P015 全链：抽取 v9.1 直落 + 离线整理 + 纯向量检索）
- 场景层去留独立拍板（蓝图 v2 场景降级为标签——organize 链尚保留过渡）
- P013 遗留：JEV 生产部署补配（决策模型上线与否随整体架构评估——准入已融进抽取 prompt）

## Deferred

- 其余列表页按批次滚动迁移——P014 事项
