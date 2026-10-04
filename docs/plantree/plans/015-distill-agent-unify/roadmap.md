# P015 roadmap

## Done

- 抽取 v9/v9.1（886b5a0/a4dc0e5/041a667）：准入合一回执 + think-off 适配（MiniMax thinking.type=disabled，
  延迟 -88% token -93%）+ few-shot 判例 + ChatRequest extras 透传通道 + 效果 demo 4 样本全对
- 落库切片（本地验证 ✅）：抽取 → conf<0.55 过滤 → 批量向量化（Qwen3-Embedding-8B 1024 维）→
  INSERT demo_atoms（pgvector + tsv）→ 回读确认。examples/extract_demo.rs 一体化管道

## In Progress

（未开始）

## Next

- T001 四个拍板点过会（阶段划分 / 检索退化代价 / 评测门槛 / 删除工具语义）
- T002 阶段一：仲裁 Agent 化（查/入库/修改/归档 四工具循环，think 关闭+强提示词，复用 P012 评测框架对照新旧仲裁）
- T003 阶段二：检索向量去留评测（纯 FTS 召回质量 vs 混合检索，数据说话）
- T004 阶段三：向量基建退役（embedding 列/索引/reembed 任务/缺向量横幅/llm_providers embedding 行）

## Deferred

- 无
