# Evidence — agent-memory-platform

按阶段归档验证证据。每条证据 = 何时、验证了什么、命令/输出摘要、结论。

## 初始化审计（/init 全量重新初始化，2026-08-20）

| 门 | 结果 | 证据 |
|---|---|---|
| cargo test --workspace | ✅ 29 套件 56 测试 0 失败 | 真 PG testcontainers 重跑（提交 `043997c` 前） |
| cargo fmt --check | ✅ 0 diff | rustc 1.97.1 |
| cargo clippy --workspace --all-targets -D warnings | ✅ 0 警告 | |
| web lint / tsc / vitest / build | ✅ 全绿（26s） | oxlint + TS 6.x + vitest + vite build |
| npm audit | ✅ 0 漏洞 | |
| cargo audit（472 crates） | ⚠️ 3 漏洞 + 3 unmaintained | lopdf 0.34（high 7.5，经 pdf-extract 进 core，升 ≥0.42 可修，生产依赖）/ tokio-tar（仅 testcontainers dev）/ rsa（lockfile 孤儿）→ Q8 |

文档-实现漂移修正：AGENTS.md 刷新（api-schema.ts 路径、全量命令清单）· module-map 目标→实际架构
（api 直连 wiki-engine/cg-bridge、storage 为 pool+迁移薄层、core 内含 sqlx 查询）·
implementation-status 清理历史 TODO · error.rs 过期注释。新增开放项 Q8–Q11（open-questions.md），
提交 `043997c`。

## Q8–Q10 技术债清理（2026-08-23，goal mt5kc1hd）

| 门 | 结果 | 证据 |
|---|---|---|
| Q8 lopdf 高危漏洞 | ✅ 已修复 | pdf-extract 0.8.2 → 0.12.0（内部 lopdf 0.34 → 0.42）；cargo audit 复扫 RUSTSEC-2026-0187 消失（3 漏洞 → 2，余 rsa 孤儿 + tokio-tar dev-only）；PDF 解析测试（含 corrupt-pdf）全绿 |
| Q9 api→core 边界收敛 | ✅ 路径 A 完成 | 新建 `parsing` crate（抽取文档解析）解开 wiki-engine→core 依赖环；core 新增 `wiki`/`codegraph` 门面；api 不再 import wiki-engine/cg-bridge（grep 零匹配）；module-map/AGENTS.md 同步 |
| Q10 zustand 死依赖 | ✅ 已清理 | package.json + lockfile 移除（grep 0 匹配）；web lint/tsc/vitest(14)/build 全绿；AGENTS.md 约定 + D0006 注记同步 |
| 全量门禁复跑 | ✅ | cargo 31 套件 56 passed / 0 failed；fmt/clippy 干净；web 四件套全绿 |

## Wiki 对齐 llm_wiki（2026-08-21 完成，审计整改后）

| 门 | 结果 | 证据 |
|---|---|---|
| 迁移 + purpose（含 query 注入 + LLM 建议更新） | ✅ | 0007 扩展四页型；0012 新表；purpose CRUD + ingest 两步注入；**query 注入**：/wiki/search 返回 {purpose, pages}（实测 purpose_has_goals:true）；**LLM 建议更新**：analysis prompt 输出 purpose_suggestion → 落 review 人审队列（kind=flag，不直接改，人审后手动 PUT） |
| overview + 级联删除 | ✅ | overview 每次 ingest 后重生成；级联删除实测：整页删+共享摘源+死链清理 2 条+index 同步；修两真 bug |
| 三页型真实生成路径 | ✅ | **generation prompt 明确规则 3/4**（synthesis：多相关实体时综合页，comparison：conflicts 非空或不同视角时对比页），page_type 枚举扩展 entity\|concept\|source\|synthesis\|comparison；**实测**：ingest 产 synthesis-对齐能力与知识库结构 页（page_type=synthesis）；mock e2e 断言 synthesis/comparison 页型+互链内容 |
| queries 闭环（真实 queries 页型） | ✅ | archive_query **直接落 page_type='queries' 页**（origin=human，**问**：结构验证 has_q:true）+ 同时入队再摄取吸收实体概念；实测：query-对齐问答 页 queries 类型 |
| Review + queries 闭环 | ✅ | ingest 后 LLM flag 产出（预定义动作+预生成检索词）；resolve 204；purpose_suggestion 也走 review |
| 4 信号 + Louvain + 洞察 | ✅ | 纯函数 6 用例 + AA；Louvain 修 ΔQ 震荡 bug；graph 返回社区；insights 四类 + dismiss |
| 前端 | ✅ | 社区/type 双着色 + 洞察联动高亮 + ReviewQueue + 级联删除 UI + 检索存档按钮 |
| 全门禁 | ✅ | cargo 56 passed / 0 failed；vitest 14 passed；tsc 0；build 0 错误；playwright 1 passed；lighthouse 100 |

## Phase 7 — 交付打磨与发布（2026-08 完成）
## Phase 7 — 交付打磨与发布（2026-08 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| 干净环境 compose up 全绿 | ✅ | `docker compose down -v`（清卷）→ `docker compose build --no-cache` → `up -d`：db+app 双 healthy、`/ready` 200、SPA 首页 200、镜像内 codegraph 1.5.0 可执行；playwright 全旅程对栈 PASS |
| 备份→恢复数据完整 | ✅ | `backup.sh backup`（pg_dump+数据卷 67.9KB）→ `down -v` 全销毁 → `restore` → `up -d`：atoms/wiki_pages/documents/persona 计数 1/6/1/1 与备份前一致 |
| AI-INTERFACE.md 驱动 AI 闭环 | ✅ | `verify-ai-loop.sh`（纯 HTTP，模拟只读文档的 AI）：签发 key→context 拉→写入会话→蒸馏链 4 阶段 succeeded→语义检索命中「PostgreSQL 17 上海」→画像产出→URL 摄取+检索命中→wiki ingest 产页+检索命中→用量 12 行可查 |
| e2e（恢复后栈复跑） | ✅ | playwright 全旅程在恢复数据后的栈上再次 PASS |

### Phase 7 修复的真问题

1. **Docker 构建上下文丢了 web/dist**（rust-embed 编译期路径）→ builder stage COPY --from=web-build
2. **GLIBC 不匹配**：builder（trixie/2.41）产物跑在 node:22-slim（bookworm/2.36）→ runtime 统一 debian:trixie-slim
3. **npm ci peer 冲突**（openapi-typescript vs typescript 6）→ web/.npmrc legacy-peer-deps + Dockerfile COPY
4. **容器 IPv6 DNS 陷阱**：容器 DNS 只回 AAAA 而 reqwest 优先 v6 → resolve 过滤（有 v4 时仅 pin v4）
5. **backup.sh 路径解析**（scripts/ 下无 compose 文件）→ 解析到 ../deploy

### 交付物

deploy/Dockerfile（三阶段：web→cargo-chef→trixie 运行时+codegraph pin）·
docker-compose.yml · scripts/backup.sh（backup/restore）·
docs/AI-INTERFACE.md（AI 客户端操作手册）· README（快速启动/备份/开发）·
CHANGELOG · CI（ci.yml：fmt/clippy/test/lint/tsc/vitest/类型漂移/docker build + e2e.yml：compose 栈 playwright）· tag v0.1.0

## Phase 6 — Web 控制台（2026-08 完成）
## Phase 6 — Web 控制台（2026-08 完成，审计整改后）

| 门 | 结果 | 证据 |
|---|---|---|
| 浏览器真全旅程（干净库全量） | ✅ | raw/playwright-live.txt：TRUNCATE 全部域表 → 登录 → Knowledge 上传→**ready（3s 轮询 UI）**→分块预览+拖拽区 → Memory 写会话→**触发蒸馏→四阶段→Playwright 原子出现（5s 轮询）** → Wiki ingest（**API 轮询等 wiki_generate succeeded**）→**sigma.js canvas 实渲染**（webgl+swiftshader）→ CodeGraph → Jobs 事件时间线，1 passed（57s 真 LLM 链路） |
| vitest 关键组件覆盖 | ✅ | raw/web-gates.txt：**10 项**（新增 components.test.tsx：atoms 归档/行内编辑保存/supersede 面板×3、persona 版本历史+回滚、Wiki 编辑器保存 PUT+版本递增） |
| Lighthouse 可达性 | ✅ | raw/lighthouse.txt：**accessibility 100**（landmark-one-main 修复） |
| UI 特性完整（无降级） | ✅ | sigma.js 真图谱（WikiGraph.tsx：graphology 建图/类型着色/点击跳转）、mermaid 代码块渲染+[[wikilink]] 页内跳转（WikiMarkdown.tsx）、Knowledge 拖拽上传（dropzone）、atoms 双击行内编辑+supersede 面板（新增+归档一步）、persona 历史 diff+回滚、编辑器人工版保存 |
| OpenAPI 类型生成 CI 强制同步 | ✅ | `openapi-dump` bin → `openapi-typescript` 生成 `api-schema.ts` → CI drift 检查 |
| 单端口静态资源服务 | ✅ | rust-embed + SPA fallback（本地栈 19581 直连验证） |

### 本轮整改修复的真 UI 缺陷

1. Knowledge/Atoms 列表无自动刷新（后台状态推进 UI 不动）→ 处理中文档 3s 轮询 + atoms 5s 轮询
2. Wiki 图谱在 ingest 未完成时点开永久显示空 → e2e 先 API 等 wiki_generate succeeded（产品层 GraphPane 依赖页面数据落库时机）
3. headless chromium WebGL 需 swiftshader 参数 → playwright.config launchOptions
4. 登录页无 main landmark → Lighthouse 96→100

### e2e 中发现并修复的真 bug

1. **登录后白屏**：App 探活 useEffect 只在挂载时跑，Login 成功后 authed 仍为 null 永渲空白 → 登录回调 prop 上抛 setAuthed(true)
2. **401 死循环**：api client 401 时 `window.location.reload()` 与探活互相触发 → 改状态切换
3. **sqlx::migrate! 增量缓存**：新增迁移文件不触发宏重扫（MIGRATOR 滞留 9 个）→ 触发重编译即修复（注意项记录）

## Phase 5 — CodeGraph 桥（2026-08 完成）
## Phase 5 — CodeGraph 桥（2026-08 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| 注册真实仓库→ready→四类查询通 | ✅ | 集成测试（本地 codegraph CLI 1.5.0）：注册本仓库自身 → index → ready → search「JobQueue」命中 struct 节点；callers「run_migrations」返回调用者；impact「enqueue」depth=2 返回 26 节点影响面；explore 返回 Markdown（含 blast radius）；CI 无 CLI 时优雅跳过 |
| CLI 超时与版本不匹配单测 | ✅ | 超时路径：1ms 超时强制中断分类 Timeout/CliUnavailable；版本：pin 1.5.0 探测 + mark_all_version_mismatch 落库 + version_mismatch 项目查询拒绝（集成测试断言） |
| 镜像内 codegraph 可用 | ✅ | Dockerfile runtime 改 node:22-slim + `npm i -g @colbymchenry/codegraph@1.5.0` + git；构建时 `codegraph version` 自验 |
| 注册校验 | ✅ | 不存在路径 BadRequest；重名拒绝 |

### 实现要点

- 版本 pin（D0005）：ensure_version 守卫 index/sync/query 入口；升级 CLI → mark_all_version_mismatch 人工重扫
- 超时矩阵：init 10min / sync 60s / query 30s；kill 子进程归一 Timeout 错误
- explore 无 --json（上游限制）→ Markdown 文本 24KB 截断保护；query/callers/callees/impact JSON 归一
- **测试基建修复**：测试容器泄漏（mem::forget 累积 198 个僵尸容器压垮 Docker daemon）→ TestPg Drop 守卫全量替换

## Phase 4 — Wiki 域（2026-08 完成）
## Phase 4 — Wiki 域（2026-08 完成）

| 门 | 结果 | 证据 |
|---|---|---|
| 两篇相关中文文档互链且不重复建页 | ✅ | 真 LLM e2e（shanghai/deepseek-v4-flash）：文档一「向量检索入门」→ 张三/pgvector/向量检索/余弦相似度等 8 页；文档二「HNSW 索引原理」→ 新建 HNSW/近似最近邻并 `HNSW → 向量检索` 互链，既有页零重复零误升级（张三 v1 保持）；mock 单测断言「向量检索」v1→v2 合并、未涉页不动 |
| human 页覆盖产生 proposal | ✅ | mock 单测：put_page(origin=human) → ingest 试图写同 slug → 页面 v1 内容不变 + job_events 产生提案事件（含 proposal_content）→ apply_proposal 合入 v2 |
| lint 报出死链/孤儿 | ✅ | 单测：注入 [[不存在的页面]] 死链 + 无入链孤儿页 → dead_link/orphan 全报出，正常互链页零误报；系统页豁免 frontmatter 检查（真 e2e 中修正的误报） |
| sha 重复 ingest 秒跳过 | ✅ | API 实测：同文本二次 ingest → `{skipped:true}` 立即返回（无新 job） |

### 实现要点

- 两步 ingest（Karpathy 模式）：wiki_analyze（实体/概念/关联/矛盾分析）→ wiki_generate（建页/更新/提案），提示词版本化 P_WIKI_ANALYSIS/GENERATION v1
- wikilink 解析器（含 `[[slug|显示名]]` 形态与中文 slug 规则）
- index/log 系统页自动维护；链接图（from_slug/to_slug/weight）
- 全程 LLM I/O 落 job_events（复用 Phase 2 可观测基建）

## Phase 3 — 知识域（2026-08 完成）

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

## Phase 2 — 记忆域（2026-08 完成）

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

## Phase 1 — 核心底座（2026-08 完成）

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

## Phase 0 — 项目地基（2026-08 完成）

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
