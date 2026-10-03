# P012-T004 · 新旧 organize 对齐评测报告

> 2026-10-03 夜间 · eval_align.rs（ignored 集成测试）· 同批 40 条生产真实原子
> 复测：`ENGRAM_EVAL_KEY=<openrouter key> cargo test -p engram-distill --test eval_align -- --ignored --nocapture`

## 输入

- 样本：生产 engram 散落原子 40 条（fixtures/eval_atoms.json），覆盖偏好/约定/事实/事件/决策五类（真实分布），全部拆散 scenario_id=NULL 独立注入两个 TestPg
- LLM：OpenRouter chat（key 走 env 不落库外）；embedding 不可用（OpenRouter 无 embedding 端点）——场景检索走 tsv 腿，embedding 留空待 reembed 补

## 结果

| 轮次 | 模型 | 旧单发 | agentic | 结论 |
|---|---|---|---|---|
| 4 | deepseek-chat-v3.1（付费） | failed（402 余额不足） | 1 场景/2 原子后 402 | 账户无 credits |
| 5 | qwen3.8-27b:free | **✅ 9 场景 / 40 原子全归属** | dead（free 限流→熔断） | 旧版基线成立；agentic 撞限流 |
| 6 | qwen3.8-27b:free | dead（配额已烧尽） | dead（同） | free 当日配额耗尽 |

### 旧单发基线（第五轮，qwen3.8-27b:free）

- **40/40 原子全部归入 9 个场景**，主题划分：
  工程偏好 / 现场交付 / 写作口吻 / 长亭认证 / 小米14 / 协作边界 / 自托管设施 / 主页展示 / 生活偏好
- 语义聚类质量人工抽验良好（偏好/设备/职业身份/项目/生活分野清晰，无混装）
- LLM 调用统计口径偏差（logs message 计数 0）——实际耗时 89.5s 证明多轮调用发生

### agentic 侧阻碍

- 循环每步一次 chat 调用（20 步上限）——free 模型限流（~20 req/min、日配额）下
  连续 429 → llm 层熔断器打开 → 任务 dead。两轮重试（对调顺序/隔窗重跑）均复现
- 非代码缺陷：生产付费模型无此限流；free 配额按账号计，当日已被评测请求消耗

## 复测指引（二选一）

1. **OpenRouter 充值 ≥$1**（deepseek-chat-v3.1 付费版，预计单次全评测 <$0.05）→ 重跑命令同上
2. **部署后生产环境评**：生产 gateway 的 embedding+chat 均可用（含向量腿检索），
   以生产真实散落原子直接 A/B（organize_agentic 开关切换）

## 初步观察（旧版基线视角）

- 旧单发在 40 原子量级下组织质量良好、成本可控（单次 prompt 全塞 ~5k tokens）
- agentic 的价值主张（解除 top100 截断、自主探索、防重复另建）需在**更大规模**
  （数百散落原子）与向量腿可用时才能体现——40 原子单发足够
- 切换策略建议（供拍板）：保持 `organize_agentic=false` 默认，生产部署 + 充值复测
  后按对照表决策；开关已就绪（settings.organize_agentic）
