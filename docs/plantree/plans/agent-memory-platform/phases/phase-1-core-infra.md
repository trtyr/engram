# Phase 1 — 核心底座

**目标**：所有域共用的基础设施达到生产质量——数据库全量 schema、任务系统、LLM 出口、鉴权、设置 API。完成后，任何域都只是「加表 + 加 job + 加路由」。

## 前置

Phase 0。

## 交付物

### 数据库迁移（分文件按域）

- [ ] 系统域：jobs、job_events、llm_providers、llm_usage、api_keys（见 storage-and-state）
- [ ] 四个域的表全部建（memory/knowledge/wiki/codegraph 各一个迁移文件——即使 Phase 2+ 才用，schema 先定版，避免后期迁移拆改）
- [ ] 索引：tsvector GIN、pgvector HNSW、外键、唯一约束（idempotency_key 等）
- [ ] embedding 维度决策落定（Q2），`vector(N)` 定死写入迁移

### jobs crate（完整实现）

- [ ] 状态机 + SKIP LOCKED 抢占 + 心跳 + visibility_timeout 回收
- [ ] 重试策略（retryable 分类 + 指数退避 + 上限）
- [ ] due_at 轮询调度器、链式入队回调
- [ ] job_events 追加 + `GET /jobs` `/jobs/:id` `/jobs/:id/events`（SSE）
- [ ] 单元测试（时钟 mock）+ testcontainers 集成测试（抢占/恢复/幂等）

### llm crate（完整实现）

- [ ] `trait LlmProvider` + OpenAI 兼容实现（chat 流式 + embedding 批量）
- [ ] provider CRUD API（key AES-GCM 加密落库）+ `/test` 连通探测
- [ ] purpose 路由（有序回退链）+ 用量记账入 llm_usage + `GET /llm/usage`
- [ ] mock provider（测试注入用）

### 鉴权与设置

- [ ] 管理员登录（密码 → 会话 token）+ API key 签发/吊销/scopes + Bearer 中间件
- [ ] `GET /openapi.json`（utoipa 基线）+ 快照测试基建

## 出口标准

1. 用 fake job kind 走完 入队→执行→重试→恢复→dead 全路径的集成测试绿
2. 配置一个真 provider → test 通过 → 一次真实 embedding 调用被记账（手测脚本留 evidence）
3. API key 三 scopes 的 401/403 行为测试绿
4. 全部迁移在干净 PG 上可重放、可回滚验证（down 或重建验证）
5. Phase 0 出口标准依然全绿

## 关联

- 设计：[jobs-system](../topics/jobs-system.md)、[llm-providers](../topics/llm-providers.md)、[api-design](../topics/api-design.md)
- 风险：R1（中文 FTS，本阶段定方案 A/C 组合）、R4、R9
