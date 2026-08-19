# Evidence — agent-memory-platform

按阶段归档验证证据。每条证据 = 何时、验证了什么、命令/输出摘要、结论。

## Phase 3 — 知识域（2026-10 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| PDF/md/URL 摄取到 ready | ✅ | 真 e2e（本地二进制 + 真网关 embedding）：`async-rust.md`（多标题分块）、手工构造合法 PDF（正确 xref 偏移）、清华镜像站 URL（标题提取「清华大学开源软件镜像站」）三类全部 ready |
| 中文检索命中 | ✅ | 「异步运行时 select 调度」命中 md 相关分块；「镜像 开源软件」命中 URL 文档；结果带 document_title + snippet + score |
| SSRF 测试集全部拒绝 | ✅ | `ssrf_fetch_rejects_private_targets`：127.0.0.1 / 169.254.169.254（云元数据）/ 10.2.0.14（内网）/ file:// 协议 / localhost 域名（DNS→环回）全部拒绝；IP 分类单测覆盖 v4 12 类 + v6 6 类；DNS pinning（reqwest resolve）防 rebinding |
| 损坏文件不阻塞队列 | ✅ | `html_ingest_and_corrupt_file_not_blocking`：损坏 PDF → failed（可读错误），后续文档照常 ready；e2e 中 GitHub 被墙 URL 同样 failed 不影响他者 |
| 重复上传秒回已有 id | ✅ | sha256 幂等（name+ct+content）：单测 deduped=true + id 相同；e2e 重复上传返回 200 + 既有文档 |
| 嵌入降级 | ✅ | 无 provider 时 chunks 标 embed_failed 但文档仍 ready（FTS 兜底）——单测（testcontainer 无网关配置）与 e2e（真网关）双路径验证 |

### e2e 中发现并修复的真 bug

1. **URL 文档 raw_path NULL 解码崩**：query_as 把 nullable 列按 String 解码 → 改 `Option<String>`
2. **摄取限流自锁**：throttle 把 pending 也计数，3 文档互相挤死重试到 dead → 移除 per-kind 限流（Runner 并发已全局约束），单用户规模正确取舍
3. **extracted 临时文件目录不存在**：写入前未 create_dir_all → 修复

### 支持格式实测

md/txt（直读）· PDF（pdf-extract，要求合法 xref）· HTML（scraper：非 script/style 元素直接文本子节点，script 天然排除）· DOCX（docx-rs：段落 + 表格）· URL（SSRF 防护 + 重定向逐跳复检 ≤3 + 20MB/30s 限制）

## Phase 2 — 记忆域（2026-10 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| 真 LLM e2e：写入→蒸馏→supersede→画像版本化 | ✅ | 本地二进制 + 本地 PG（shanghai/deepseek-v4-flash + Qwen3-Embedding-8B@1024）。两轮蒸馏：轮1「住在上海」入库；轮2「搬到北京」→ 旧原子 superseded(superseded_by 链)、新原子 active、`开发环境`场景 v2 更新、identity 画像 **v1(上海)→v2(北京)**，`GET /memory/persona/history?aspect=identity` 双版本可 diff |
| /memory/context 三层引用链 | ✅ | `?query=用户住在哪个城市`：L3=3 分面 + L2=2 场景(开发环境/沟通偏好) + L1=3 原子，441 chars 未截断；persona.evidence_refs → scenario → atoms → sessions 逐级可回溯（jsonb_pretty 验证） |
| mock 单测：解析重试/仲裁三分支/版本化 | ✅ | distill 3 项：非法 JSON→追加指令重试成功；new/duplicate(删+hit_count)/contradicts(superseded+链) 三分支真实 id 验证；persona v1→v2 + evidence 链 |
| 中文检索（jieba 预分词 + RRF） | ✅ | `用户住在哪个城市`：L1 命中「已从上海搬到北京居住」（superseded 不返回）、L2 命中「用户现居北京」；search 集成测试 11 条中文样本全命中 |
| LLM I/O 可观测（用户新增要求） | ✅ | 每次调用完整 input/output 记入 job_events（含重试标记与错误）；e2e 中 6 次调用全部可回放 |
| compose 门禁保持 | （见下） | |

### e2e 中发现并修复的真 bug

1. **resolve 回退不按能力选模型**：未配路由时 Embed 用途可能回退到 chat 模型（网关报 messages 无效）→ 修复：按 purpose 能力匹配（Embed→embedding，chat→非 embedding）
2. **organize 提示词缺 update 格式**：模型不知道 update 要带 scenario_id，冲突场景返回空动作 → 修复：补 update 输出格式说明
3. **extract 失败不回滚会话状态**：会话滞留 processing 导致重试空转假成功 → 修复：Err 时批量退回 pending
4. persona 空画像不产出初始分面 → 提示词补规则 2
5. MockLlm 向量维度与存储不一致（8 vs 1024）→ 统一 1024

### 环境

- 网关模型：`shanghai/deepseek-v4-flash`（glm-5.3 通道在 e2e 时段 502，用户指定换 shanghai）
- 复现：`scripts/verify-memory-e2e.sh <base_url> <key> shanghai/deepseek-v4-flash Qwen/Qwen3-Embedding-8B`
  （脚本已更新为两轮蒸馏模式；本地手动验证记录见上）

## Phase 1 — 核心底座（2026-10 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| 全量迁移干净 PG 可重放 | ✅ | `migrations_apply_on_clean_pgvector`：10 份迁移应用 + 幂等重放；`all_domain_tables_exist_with_columns`：16 表存在、vector(1024) 列型、状态机 CHECK、幂等键唯一约束全部验证 |
| fake job 全生命周期 | ✅ | jobs 集成测试 5 项：成功路径（入队→抢占→完成→事件）/重试退避→dead/永久失败直接 failed/dead 复活/幂等键去重/僵尸回收/Runner 真执行（double x21→42） |
| 真实 provider 连通 + embedding 记账 | ✅ | `scripts/verify-real-provider.sh`（对真网关）：chat 1173ms + embed 480ms×4096 维；llm_usage 记账 2 行（4+13 tokens）；密钥密文落库（明文不在 DB） |
| API key 401/403 行为 | ✅ | `auth_401_403_matrix`：无凭证/坏 key/坏会话→401；错密码→401；管理员全通；API key 读 jobs 通、settings/usage→403；完整 key 不回显 |
| OpenAPI 快照 | ✅ | `openapi_snapshot`：13 端点集合快照，变更必须显式更新 |
| mock provider 单测 | ✅ | crypto（加解密往返/错主密钥拒绝）×3 + 路由表 roundtrip + HTTP 错误分类（连接拒绝→瞬态） |
| compose 起栈（Phase 0 标准保持） | ✅ | Phase 1 代码重建镜像→起栈：/ready 200、/auth/login 颁发 token、/jobs 无凭证 401 |

### 关键实现事实

- Runner 优雅停机：watch channel + select 空转轮询可中断
- test_provider 双探测：chat + embedding（有对应能力模型时各发一次探测，均记账）
- EmbedRequest.dimensions（matryoshka 降维）：网关 Qwen3-Embedding-8B 原生 4096 维，
  传 dimensions=1024 适配存储层 vector(1024)（D0010 修订：默认 embedding 通道 = Qwen3-Embedding-8B@1024）
- 真网关：https://newapi.trtyr.top（用户 newapi，key 不落仓库，仅运行时注入）

## Phase 0 — 项目地基（2026-10 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| cargo fmt --check | ✅ 0 diff | 本地 rustc 1.97.1 |
| cargo clippy --workspace --all-targets -D warnings | ✅ 0 警告 | 同上 |
| cargo test --workspace | ✅ 1 passed | `migrations_apply_on_clean_pgvector`：testcontainers 起 pgvector/pgvector:pg17 → 迁移应用 → `SELECT '[1,2,3]'::vector` 成功 → 幂等重放成功 |
| web tsc --noEmit | ✅ 0 错误 | TS 6.x |
| web oxlint | ✅ 仅 shadcn 生成代码已知 fast-refresh 警告 | |
| web npm run build | ✅ 通过 | Vite 7 + Tailwind v4 + shadcn(radix) |
| docker compose build | ✅ 镜像构建成功 | cargo-chef 分层缓存；首次依赖编译约 4 分钟 |
| docker compose up + 探针 | ✅ 全部 200 | `/health`→`{"status":"ok"}`；`/ready`→`{"status":"ready","migration_version":1}`（容器内自动迁移）；`/openapi.json` 正常；未知路由统一错误体 404 |

### 环境备注（本机）

- 本机 8080 与 18080 先后被其他项目占用，本地验证固定用 `AGENT_MEMORY_PORT=19180`（起栈前先 `lsof` 确认空闲）
- Docker daemon：OrbStack；buildx 首次解析基础镜像偶发 auth.docker.io 超时
  → 解法：`docker pull node:22-slim lukemathwalker/cargo-chef:latest-rust-1.97-slim debian:bookworm-slim` 预拉后重建
- CI（GitHub Actions ubuntu-latest）不受上述本机问题影响
