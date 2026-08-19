# Topic — LLM 提供方与路由

平台所有 LLM 调用的唯一出口（`llm` crate）。蒸馏/Wiki/embedding 不各自直连。

## Provider 模型

```text
llm_providers: id, name, base_url(OpenAI 兼容), api_key_encrypted,
               models: [{id, capabilities: [chat|embedding]}], is_default
```

- 加密：AES-256-GCM，主密钥来自 env `AGENT_MEMORY_MASTER_KEY`（部署生成，丢失则需重录 key）
- test-connection：发 1-token 探测请求，返回延迟与模型列表

## 用途路由

`purpose → [(provider, model), ...]` 有序回退链，管理员可配：

| purpose | 默认档位 |
|---|---|
| extract / arbitrate / embed | 低档（便宜快，如 deepseek-chat / bge 类） |
| organize / consolidate / wiki_analysis | 中档 |
| persona / wiki_generation | 高档（质量敏感） |

未配置的 purpose → 默认 provider 的第一个 chat 模型。

## 用量记账

每次调用写 `llm_usage`：purpose、model、input/output tokens、latency、job_id、ts。

- `GET /llm/usage` 聚合视图（按日/用途/模型），UI Dashboard 消费
- 单 job token 预算上限（蒸馏链熔断，见 distill-pipeline）

## 客户端实现

- reqwest + SSE 流式（chat）；embedding 批量接口
- 超时：chat 120s / embed 30s；连接池复用
- trait `LlmProvider` 注入——测试用 mock provider，管道测试零真实调用
