# 架构

## 目录树与模块边界

后端是 `server/` 下的 Rust workspace，10 个 crate，边界按「层 + 域」切分：

```text
server/
├── Cargo.toml              # workspace 定义 + workspace.dependencies
├── Cargo.lock
├── migrations/             # schema 唯一定义处（12 个迁移，启动自动执行）
└── crates/
    ├── api/                # HTTP 层（唯一有 main 的 crate）
    ├── core/               # 领域编排层（域服务 + 门面）
    ├── storage/            # 持久化薄层：连接池装配 + 迁移执行
    ├── llm/                # LLM 客户端：provider 抽象 + 路由 + 记账 + 加密
    ├── jobs/               # 任务系统：PG-backed 队列 + worker + 事件
    ├── distill/            # 蒸馏管道：L0→L1→L2→L3 编排 + 版本化提示词
    ├── search/             # 混合检索：FTS + 向量 + RRF 融合
    ├── wiki-engine/        # Wiki 引擎：两步 ingest + wikilink + lint + 图
    ├── cg-bridge/          # CodeGraph 桥：CLI 子进程包装
    └── parsing/            # 文档解析：pdf/docx/html/md/txt → 纯文本
```

## 依赖方向

以各 crate 的 `Cargo.toml` 为准（`[dependencies]` 里声明的内部 crate）：

```text
api ─────────→ storage, jobs, llm, core, distill     (装配：连接池/Runner/域服务)
core ────────→ jobs, llm, search, distill, parsing, wiki-engine, cg-bridge
wiki-engine ─→ jobs, llm, search, distill, parsing
distill ─────→ jobs, llm, search

叶子（无内部依赖）：
  llm, jobs, search, cg-bridge, parsing, storage
```

规则：

- **api 只经 `core` 访问域服务**。wiki 与 codegraph 在 `core` 里是纯门面（`core::wiki` / `core::codegraph` 直接 re-export `wiki-engine` / `cg-bridge` 的类型），api 不直连这两个 crate。
- **`storage` 是 pool + 迁移薄层**，只被 `api`（main.rs 装配时）使用；`core` 等 crate 拿 `sqlx::PgPool` 直接写 SQL，没有建仓储抽象层。
- **禁止域 crate 之间横向 import**（如 `distill` → `wiki-engine`）；跨域编排放 `core`。
- **`parsing` 是独立底层 crate**，`core`（知识域）与 `wiki-engine` 摄取共用，用来解开原来的 api→core 依赖环。

## 各 crate 职责

### api — HTTP 层

服务入口与接口面。`src/main.rs` 是薄壳：`Config::from_env` → 连接池 → 迁移 → 注册 job handler → 路由 → 监听。

| 文件 | 职责 |
|---|---|
| `main.rs` | 启动编排（配置→池→迁移→Runner 装配→HTTP 服务→优雅停机） |
| `lib.rs` | crate 根，模块声明 |
| `config.rs` | 环境变量加载（`AGENT_MEMORY_*` 前缀） |
| `state.rs` | `AppState` 依赖注入容器（pool + admin_password + master_key + data_dir） |
| `auth.rs` | 鉴权：admin session（`ams_` 前缀 opaque token）+ API key（`amk_` 前缀，scopes）；Bearer 中间件 |
| `error.rs` | 统一错误体 `{"error":{code,message,retryable}}` + `ApiError` 分类 |
| `routes/*.rs` | 8 个路由模块（health/auth/jobs/llm/memory/knowledge/wiki/codegraph） |
| `web_assets.rs` | SPA 静态资源兜底（`rust-embed` 内嵌 `web/dist`） |
| `bin/openapi-dump.rs` | 从代码提取 openapi.json 的 CLI（CI 用，不起服务） |

### core — 领域编排层

域服务与跨域编排。`src/lib.rs` 导出 `MemoryService`、`KnowledgeService`、`WikiService`（re-export）、`CgBridge`（re-export）。

| 文件 | 职责 |
|---|---|
| `memory.rs` | 记忆域：L0 写入/触发、检索、上下文包、L1 治理、L3 画像视图 |
| `knowledge/mod.rs` | 知识域：文档摄取编排 + 检索 |
| `knowledge/pipeline.rs` | 摄取 job handlers：parse → chunk → embed 三步链 |
| `knowledge/chunking.rs` | 结构感知分块（标题优先，目标 ~800 字符，重叠 15%） |
| `knowledge/ssrf.rs` | URL 摄取的 SSRF 防护 |
| `wiki.rs` | Wiki 域门面（re-export `wiki-engine`） |
| `codegraph.rs` | CodeGraph 域门面（re-export `cg-bridge`） |

### storage — 持久化薄层

| 文件 | 职责 |
|---|---|
| `pool.rs` | `PoolConfig` + `connect_pool`（sqlx PgPool 装配） |
| `migrate.rs` | `run_migrations` / `current_version`（执行 `migrations/` 目录） |

### llm — LLM 客户端（平台所有 LLM 调用的唯一出口）

| 文件 | 职责 |
|---|---|
| `provider.rs` | `LlmProvider` trait + OpenAI 兼容实现 + `ProviderRegistry`（含用量记账） |
| `router.rs` | purpose 路由：用途 → (provider, model) 有序回退链（规则存 `settings` 表） |
| `crypto.rs` | `KeyCipher`：AES-256-GCM 加解密 provider API key |
| `types.rs` | 请求/响应/错误/用途类型 |

### jobs — 任务系统

| 文件 | 职责 |
|---|---|
| `queue.rs` | `JobQueue`：入队、抢占（SKIP LOCKED）、心跳、完成/失败、回收、事件、查询 |
| `runner.rs` | `Runner`：轮询抢占 + 执行 handler 的 worker 池；`JobContext` 提供进度/事件/链式入队 |
| `types.rs` | `Job` / `JobStatus` / `JobTemplate` / `JobEvent` 类型 |

状态机：`pending → running → succeeded | failed(可重试→pending) | dead(重试耗尽)`。

### distill — 蒸馏管道（L0→L3）

每个阶段是独立 job，成功后显式入队下游（ID 链通过 payload 传递）：

| 文件 | 职责 |
|---|---|
| `extract.rs` | L0 会话 → 候选 L1 原子 |
| `arbitrate.rs` | 候选 × 既有相似 → 新增 / 去重 / 矛盾取代 |
| `organize.rs` | 未归组 L1 → 新建/更新 L2 场景块 |
| `persona.rs` | 变动 L2 → L3 画像分面新版本（版本化 + 证据链） |
| `consolidate.rs` | 近重复合并 + stale 降权（每周定时 + 手动） |
| `chain.rs` | handler 注册 + 防抖触发 |
| `prompts.rs` | 版本化提示词 `PromptId(name, version)` |
| `llm_port.rs` | `DistillLlm` / `GatewayLlm` 抽象 |

### search — 混合检索

| 文件 | 职责 |
|---|---|
| `hybrid.rs` | 单条 SQL 内 FTS + ANN + RRF 融合（`RRF_K=60`） |
| `rrf.rs` | RRF 融合算法 |
| `tokenize.rs` | 中文 jieba 预分词 → tsvector 查询/文本 |

### wiki-engine — Wiki 引擎

| 文件 | 职责 |
|---|---|
| `ingest.rs` | 两步 ingest job handlers：`wiki_analyze → wiki_generate` |
| `service.rs` | `WikiService`：页面 CRUD、检索、门面 |
| `markup.rs` | wikilink 解析 + slug 校验 |
| `lint.rs` | lint（产生 `LintReport`） |
| `cascade.rs` | 级联更新（`CascadeReport`） |
| `community.rs` | 社区/聚类相关 |
| `relevance.rs` | 相关性判断 |
| `review.rs` | review 系统（`ReviewItem`） |
| `insights.rs` | 洞察（`InsightsReport`） |
| `purpose.rs` | purpose 配置 |
| `prompts.rs` | 版本化提示词 |

### cg-bridge — CodeGraph 桥

| 文件 | 职责 |
|---|---|
| `bridge.rs` | `CgBridge`：包装 `codegraph` CLI（子进程 + `--json`），注册/同步/查询代理与错误归一 |

### parsing — 文档解析

| 文件 | 职责 |
|---|---|
| `lib.rs` | `detect_format` + `parse_bytes`：pdf/docx/html/md/txt → 纯文本 |

## 运行时装配（main.rs 的启动顺序）

1. `Config::from_env()` 读环境变量（必填 `AGENT_MEMORY_DATABASE_URL`）。
2. 结构化 JSON 日志（`tracing-subscriber` + `EnvFilter`）。
3. `connect_pool` → `run_migrations`（启动即迁移，失败快速退出）。
4. `Runner` 装配三段 handler：`distill::register_handlers`（蒸馏链）→ `core::knowledge::register_handlers`（知识摄取）→ `core::wiki::ingest::register_handlers`（wiki 摄取）。
5. `AppState::new(pool)` → `routes::router(state)` → 监听 `0.0.0.0:port`。
6. 优雅停机：先等 HTTP 再停 runner（响应 SIGTERM/SIGINT）。

## 数据流

所有长操作走 job 队列；LLM 调用走 `llm` crate；检索走 `search` crate；文档/Wiki 原料解析走 `parsing` crate。数据落 PostgreSQL（详见 [data-model.md](data-model.md)）。完整 schema 见 `server/migrations/` 的 12 个迁移文件。
