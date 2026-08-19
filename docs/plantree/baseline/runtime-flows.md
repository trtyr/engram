# Runtime Flows — 核心运行时流程

> 目标设计。三条主流程 + 一条横切流程。

## 流程 1：记忆写入与蒸馏（Chat Memory 主链路）

```text
AI 客户端                     平台 (Rust)
   │                             │
   ├─ POST /memory/sessions ────► 写 L0 raw_sessions（不可变）
   │                             │ 触发条件满足（手动 / debounce / 定时）
   │                             ▼
   │                        jobs 入队 extract_atoms (L0→L1)
   │                             │ LLM 抽取：偏好/事实/决定/事件
   │                             │ 与既有 L1 比对：新/重复/矛盾
   │                             │ 矛盾 → 旧条目标 superseded，保留历史
   │                             ▼
   │                        jobs 入队 organize_scenarios (L1→L2)
   │                             │ 按主题聚类成场景块（增量，不全量重建）
   │                             ▼
   │                        jobs 入队 distill_persona (L2→L3)
   │                             │ 增量更新画像 aspect，版本化
   │                             ▼
   ├─ GET /jobs/:id ◄──────── 任务状态/事件可查（SSE 可选）
   │                             │
   ├─ POST /memory/search ─────► 分层检索：L2/L3 先行（快启动上下文）
   │                             │ 需要精确事实时回退 L1/L0
   │                             │ BM25 + 向量 + RRF，预算封顶
```

**要点**：写入永不阻塞蒸馏（异步 job）；蒸馏阶段各自幂等可重试；每阶段产物带来源引用链（L3 → L2 → L1 → L0），画像里每句话可溯源。

## 流程 2：知识摄取（Knowledge）

```text
POST /knowledge/documents (文件/URL)
  → 解析（md/pdf/docx/html…，流式）→ 分块 → sha256 去重
  → 嵌入（可配 embedding provider）→ 入库 chunks
  → 状态机：pending → parsing → chunking → embedding → ready / failed
检索：POST /knowledge/search → FTS + 向量 + RRF → 带 document 引用
```

## 流程 3：Wiki 编译（两步 ingest，Karpathy 模式）

```text
POST /wiki/ingest (source 文档引用)
  → sha256 缓存检查（未变则跳过）
  → Job A analysis：LLM 读 source + 现有 wiki index
      产出：实体/概念清单、与现有页的关联、矛盾点、结构建议
  → Job B generation：LLM 按 analysis 生成/更新页面
      产出：页面（frontmatter 带 sources[]）、[[wikilink]]、
            更新 index.md / log.md / overview.md
  → 嵌入新页 → 图数据更新（wiki_links）
人工：UI 浏览页面/diff，可编辑纠偏（人审 LLM 维护）
维护：POST /wiki/lint → 死链/孤立页/过时检测报告
```

## 流程 4：CodeGraph 代理（横切）

```text
POST /codegraph/projects (本地路径或 git URL)
  → cg-bridge spawn `codegraph init/sync`（子进程，超时控制）
GET  /codegraph/projects/:id/status ← 解析 CLI --json 输出
POST /codegraph/query { kind: explore|callers|callees|impact|search }
  → 包装 CLI 调用 → JSON 归一化 → 返回
```

## 横切：任务系统（所有长操作走这里）

- job 记录：kind / payload / status(pending|running|succeeded|failed) / attempts / error / progress / 幂等键
- worker：进程内 tokio 任务池 + PG 行锁抢任务；崩溃恢复（running 超时回收）
- 事件：job_events 表追加，`GET /jobs/:id/events` 轮询或 SSE
- 重试：仅标记 retryable 的失败，指数退避，最多 3 次

## 横切：LLM 调用约定

- 所有 LLM 调用走 `llm` crate：provider 抽象 + 任务路由（extract→便宜模型，synthesize→强模型）
- 提示词模板在代码内版本化（PROMPT_VERSION 常量），蒸馏产物记录所用版本
- 每次调用记 usage（tokens/耗时/模型）入 llm_usage 表，UI 可查成本
