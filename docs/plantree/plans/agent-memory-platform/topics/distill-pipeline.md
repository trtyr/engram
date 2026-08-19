# Topic — 蒸馏管道

内置 LLM 蒸馏（D0002）。管道 = 一组 jobs + 版本化提示词 + 模型路由。

## 阶段

| 阶段 | 输入 | 输出 | 模型档位 | 提示词 |
|---|---|---|---|---|
| extract_atoms | L0 会话（新） | 候选 L1 + 与既有 atoms 的关系判定 | 低档（便宜快） | P_EXTRACT_v1 |
| arbitrate | 候选 L1 × 既有相似 atoms | 重复/矛盾/新知 判定 | 低档 | P_ARBITRATE_v1 |
| organize_scenarios | 未归组 L1 + 既有 L2 | 新建/更新 L2 | 中档 | P_ORGANIZE_v1 |
| distill_persona | 变动过的 L2 + 既有 L3 | L3 各 aspect 新版本 | 高档（质量敏感） | P_PERSONA_v1 |
| consolidate | 全库老化数据 | 合并/降权/重聚类 | 中档 | P_CONSOLIDATE_v1 |

- 档位由路由规则映射到具体 provider/model（见 [llm-providers.md](llm-providers.md)）。
- 每个产物记录 `prompt_version + model + usage`，可归因可回放。

## 触发策略

1. **自动**：L0 写入后 debounce（默认 30s，可配）聚合新会话触发 extract→…→persona 链
2. **手动**：`POST /memory/distill`
3. **定时**：consolidate 每周；触发器用 PG 侧调度（jobs 表 due_at 轮询，不引入外部 cron）

链式执行：extract 成功 → 仲裁 → organize（仅有新 L1 时）→ persona（仅 L2 有变动时）。每步独立 job，失败不阻塞上游产物。

## 失败语义

- LLM 调用：429/5xx/超时 → retryable，指数退避重试（≤3）
- 输出解析失败 → retryable 一次（带「严格 JSON」追加指令）；再失败 → job failed + 事件留痕，人工可在 UI 重跑
- 单次蒸馏链成本上限（token 预算参数，超限熔断标记 failed_cost_limit）

## 提示词工程约定

- 全部提示词为代码内常量 + 版本号；修改必须升版本
- 输出强制 JSON schema（响应中约束 + 解析校验，失败即重试路径）
- 中文语料中文提示词；所有抽取输出语言跟随源（Q5 同款约定）

## 质量守护

- 抽取置信度低于阈值的 atom 标记 `needs_review`，UI 提供人审队列（只标记不阻塞）
- 画像 diff 在 UI 可视，人可一键回滚 aspect 版本
