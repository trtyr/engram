# 🧠 Engram

**单用户 AI 长期记忆平台** —— 把 AI 的记忆做成可蒸馏、可检索、可审计、可遗忘的资产

![Rust](https://img.shields.io/badge/Rust-orange?style=for-the-badge&logo=rust&logoColor=white)
![React](https://img.shields.io/badge/React-19-blue?style=for-the-badge&logo=react&logoColor=white)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-17-336791?style=for-the-badge&logo=postgresql&logoColor=white)
![pgvector](https://img.shields.io/badge/pgvector-HNSW-16c784?style=for-the-badge)
![License](https://img.shields.io/badge/license-MIT-green?style=for-the-badge)

> **en·gram**（/ˈenɡræm/）＝神经科学里的「记忆痕迹」——记忆在脑中留下的物理印记。
> AI 的记忆，不该随会话蒸发。

---

## ✨ 它是什么

Engram 是一个「**平台即工具**」：平台对外暴露 HTTP API，AI（或人）拿着 API key 操纵；人通过 Web 控制台管理浏览。核心理念——**信任源于可溯源**：每一层记忆都能回放它从哪来、怎么来的。

## 🪜 记忆蒸馏阶梯

```text
L0 会话 ──蒸馏──▶ L1 原子 ──组织──▶ L2 场景 ──沉淀──▶ L3 画像
（原文）         （事实单元）       （场景模式）       （prompt 注入用画像）
```

- 🔍 **全程可溯源**：蒸馏链每层记录 `prompt_version`，任意记忆可归因回放到产出它的那一版 prompt。
- 🧭 **一坐标系**：entities（人物/项目/主题/群组/地点）由蒸馏自动抽取，横向切记忆的透镜；用户本人是所有记忆的 owner。

## 🗂️ 五域资产

| 域 | 记什么 | 形态 |
|:--|:--|:--|
| 💬 **Chat Memory** | 用户的事实、偏好、决策、事件 | L0→L3 分层蒸馏，全程可溯源 |
| 🕸️ **Wiki** | 世界的知识：文档 + LLM 增量互链 | 文档/URL 摄取→检索（jieba FTS + pgvector ANN + RRF）+ 多源摄取→生成→图谱 + lint 死链分级 |
| 🧬 **CodeGraph** | 代码库结构索引 | 符号/调用关系六种结构化查询 |
| 🧩 **项目记忆** | 跨会话的工作「线」 | 项目 CRUD + 多主机位置 + 分类文档 |
| 🧩 **Skills** | 可复用的 AI 指令包（SKILL.md 形态） | slug 唯一 + frontmatter 容错导入（含 `>-`/`|` 块标量，`scripts/import-skills.sh` 目录一键灌入）+ 版本快照回滚 + 全量导出 |

## 🛡️ 治理能力

- 🔒 **敏感标记** —— 检索/打包/导出默认排除，快照层同生共死
- 🧹 **一等清空** —— deep purge 两阶段 + 确认短语，agent 级彻底清场（会话物理删除）
- ⚖️ **编辑分权** —— AI 禁改语义内容（走会话→蒸馏），用户改动留痕钉住
- 🧾 **数据主权** —— 全量导出，密钥 AES-GCM 加密落库

## 🏗️ 技术形态

```text
Rust (axum) 单二进制 ──同源托管──▶ React 19 SPA（Engram 控制台）
        │
        └── PostgreSQL + pgvector；一切 LLM 长操作走 PG 任务队列（死信/重试/事件）
```

- **后端**：Rust workspace 10 crates（`engram-api` / `engram-core` / `engram-storage` / `engram-wiki-engine` / …），axum + sqlx + pgvector，rust-embed 托管前端。
- **前端**：React 19 + TypeScript + Vite 8 + Tailwind 4，十页 SPA（概览/用户记忆/圈子/Wiki/代码图谱/项目/技能/任务/MCP/设置），墨白双主题。
- **单二进制单端口**：一个 `engram-server` 交付前后端，本地/Docker 单机部署。

## 🚀 快速上手

### Docker（推荐）

```bash
cd deploy
docker compose up
# 控制台 http://localhost:8080，管理员密码见 .env
```

### 本地开发

```bash
# 后端（需 PostgreSQL + pgvector）
cd server
cargo build --bin engram-server
AGENT_MEMORY_DATABASE_URL=postgres://127.0.0.1:5432/engram \
AGENT_MEMORY_ADMIN_PASSWORD=admin123 \
AGENT_MEMORY_MASTER_KEY=$(python3 -c "print('ab'*32)") \
  ./target/debug/engram-server

# 前端（dev server）
cd web && pnpm install && pnpm dev
```

## 🔌 MCP 接入（用户记忆域）

engram-server 内置 MCP（Model Context Protocol）服务端（官方 Rust SDK `rmcp`，Streamable HTTP），
让 Claude Code / Cursor / Claude Desktop 等 AI 客户端直接操纵你的用户记忆：

- **端点**：`http://<host>:8080/mcp`（鉴权：`Authorization: Bearer amk_…`，按域 scope：memory / project / skills / wiki）
- **二十三个工具（memory 九 + skills 六 + wiki 八）**：`memory_context`（冷启动上下文包）/ `memory_search` / `memory_list_atoms` /
  `memory_list_sessions` / `memory_get_session` / `memory_write_session` / `memory_append_session` /
  `memory_forget` / `memory_entities`；`skills_list` / `skills_get` / `skills_create` / `skills_update` /
  `skills_delete` / `skills_import`——instructions 与工具描述写明调用时机与编辑分权
  （AI 只写会话，蒸馏沉淀为 L1~L3；改写语义内容是用户专属）
- **Wiki 域八工具**：`wiki_search`（混合检索 + purpose）/ `wiki_list_pages` / `wiki_get_page` /
  `wiki_write_page`（AI 通道写页，frontmatter 落 via="ai"）/ `wiki_ingest`（异步织入）/
  `wiki_archive_query`（问答存档）/ `wiki_graph` / `wiki_lint`
- **管理**：控制台「MCP」页——服务总开关（关闭即整体 503）、按域逐个看工具开关
  （停用即对 AI 隐身 + 调用拒绝）；MCP 专用密钥在
  「设置 → API 密钥」签发（scope 选择器勾 memory / project / skills / wiki）
- **检索质量**：FTS 零命中时向量腿收紧阈值（不相关查询返回空而非噪声页）+ 查询侧
  领域停用词；阈值按 embedding 模型用 `AGENT_MEMORY_VEC_FALLBACK_MAX_DISTANCE` 调整
  （Qwen3-Embedding-8B 建议 0.65），非对称检索模型（Qwen3）需同时设
  `AGENT_MEMORY_EMBED_QUERY_INSTRUCTION`（查询侧指令，文档侧不包装）

Claude Code 快速接入：

```bash
claude mcp add --transport http engram http://localhost:8080/mcp \
  --header "Authorization: Bearer amk_你的密钥"
```

> 远程部署需放开 Host 白名单：`AGENT_MEMORY_MCP_ALLOWED_HOSTS=mem.example.com`（默认仅 loopback，防 DNS rebinding）。

门禁（当日全绿命令）：

```bash
cd server && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build
```

## 📖 文档

三层档案，本目录是全栈集成视角，后端/前端各有独立档案：

| 层 | 入口 | 视角 |
|:--|:--|:--|
| 全栈 | [docs/](docs/README.md) | 仓库整体、前后端接缝、跨栈约定 |
| 后端 | [server/docs/](server/docs/README.md) | Rust workspace 完整档案 |
| 前端 | [web/docs/](web/docs/README.md) | Engram SPA 完整档案 |

- 📐 产品事实与设计系统：[PRODUCT.md](PRODUCT.md)、[DESIGN.md](DESIGN.md)
- 🧭 接手第一步读 [docs/current-state.md](docs/current-state.md)

---

*Made with 🧠 — **Engram**，记忆，不该蒸发。*
