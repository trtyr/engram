<div align="center">

# 🧠 Engram

### 把 AI 的记忆，做成可蒸馏、可检索、可审计、可遗忘的资产

**单用户 AI 长期记忆平台 · Rust 单二进制 · 七域 MCP 渐进式发现 · Wiki 多库 · 全程可溯源**

[![Rust](https://img.shields.io/badge/Rust-axum-DEA584?style=for-the-badge&logo=rust&logoColor=white)](server/)
[![React](https://img.shields.io/badge/React_19-SPA-61DAFB?style=for-the-badge&logo=react&logoColor=black)](web/)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL_17-pgvector-4169E1?style=for-the-badge&logo=postgresql&logoColor=white)](server/crates/storage/)
[![MCP](https://img.shields.io/badge/MCP-七工具_63操作_渐进式发现-8A2BE2?style=for-the-badge)](#-mcp-七工具工具面渐进式发现)
[![Tests](https://img.shields.io/badge/tests-243_cargo_·_59_vitest-16C784?style=for-the-badge)](#-门禁)
[![License](https://img.shields.io/badge/license-MIT-3DA639?style=for-the-badge)](LICENSE)

> **en·gram**（/ˈenɡræm/）*n.* 神经科学中的「记忆痕迹」——记忆在脑中留下的物理印记。
>
> **AI 的记忆，不该随会话蒸发。**

</div>

---

## 💥 为什么需要它

大模型的记忆活在一个会话里：关掉窗口，用户是谁、项目推进到哪、上次踩过什么坑——全部清零。
给模型塞一个「记事本」解决不了问题，因为**记忆的质量取决于记忆的治理**：

- 塞进去的东西可信吗？——**每一层都要能回放它从哪来、怎么来的**
- 记错了怎么办？——**纠错走取代链，而不是覆盖**
- 不想让它记的，真忘得掉吗？——**遗忘是一等公民，不是 UPDATE 语句**

Engram 是一个「平台即工具」：平台对外暴露 HTTP API，AI（或人）拿着 API key 操纵；
人通过 Web 控制台治理浏览。核心理念一句话——

> ## 信任源于可溯源 🔍

## 🪜 记忆蒸馏阶梯

对话原文不是记忆，记忆是被蒸馏出来的。Engram 把「一句闲聊」变成四层可治理的资产：

```mermaid
flowchart LR
    L0["💬 L0 会话<br/><i>原始对话</i>"] -->|蒸馏| L1["⚛️ L1 原子事实<br/><i>可检索的最小单元</i>"]
    L1 -->|组织| L2["🌫️ L2 场景模式<br/><i>同类事实的沉淀</i>"]
    L2 -->|沉淀| L3["👤 L3 用户画像<br/><i>prompt 注入用</i>"]
    L3 -.->|冷启动注入| AI["🤖 你的 AI"]
    L1 -.->|定向回忆| AI
```

- **全程可溯源**：蒸馏链每层记录 `prompt_version`，任意记忆可归因回放到产出它的那一版 prompt
- **纠错走蒸馏**：把正确的表述写成对话，蒸馏自动生成取代链——永不直接改写语义内容
- **遗忘是断层**：memory 域 `forget`（void）会话作废，已蒸馏产物**级联归档**，检索立即失效
- **实体坐标系**：人物 / 项目 / 主题 / 群组 / 地点，由蒸馏自动抽取，横向串联所有记忆

## 🗂️ 七域资产

| 域 | 记什么 | 形态 |
|:--|:--|:--|
| 💬 **Chat Memory** | 用户的事实、偏好、决策、事件 | L0→L3 分层蒸馏，全程可溯源 |
| 🕸️ **Wiki** | 世界的知识：文档 + LLM 增量互链 | 文档/URL 摄取 → `[[wikilink]]` 知识网 + 图谱 + lint 死链分级 |
| 🧬 **CodeGraph** | 代码库结构索引 | 本地路径/git 注册 → CLI 异步索引 → 六种结构化查询 + 调用子图 / 文件依赖全图 |
| 🧩 **项目记忆** | 跨会话的工作「线」 | 项目 + 多主机位置 + 分类文档，精确寻址读（零截断） |
| 🪄 **Skills** | 可复用的 AI 技能包（文件夹：SKILL.md + scripts/references） | slug 唯一 + 容错导入 + 版本快照回滚 + 附属文件按路径寻址 + 三层取用 |
| ✅ **待办** | 不绑定项目的快速待办（灵感/计划/操作/排查） | open/done/archived + 优先级 + 标签 + 截止时间，全局检索直达 |

## 🛡️ 治理，不是摆设

| | 能力 | 一句话 |
|:--|:--|:--|
| 🔒 | **敏感标记** | 医疗/感情/财务对话一个开关，检索/打包/导出默认排除 |
| ⚖️ | **编辑分权** | AI 只写会话；改写语义内容是用户专属，改动留痕钉住 |
| 👤 | **账号与会话** | 管理员账号（用户名+密码 PBKDF2）+ 会话列表吊销；登录页首次使用引导创建账号 |
| 🧹 | **一等清空** | deep purge 两阶段（arm 5 分钟冷却 → token 执行），agent 级彻底清场 |
| 🧾 | **数据主权** | 全系统一键导出/导入 + 远程拉取迁移（A→B）；密钥 AES-GCM 加密落库 |
| 🚦 | **任务队列** | 一切长操作走 PG 队列（蒸馏/摄取/索引/同步），pending/running/dead 生命周期可见可恢复 |

## 🏗️ 技术形态

```text
                    ┌─────────────────────────────┐
   AI 客户端 ──MCP──▶│                             │
                    │   engram-server（Rust 单二进制）│
   浏览器 ────HTTP──▶│   axum + sqlx + rust-embed   │
                    └──────────────┬──────────────┘
                                   │
                    ┌──────────────▼──────────────┐
                    │  PostgreSQL 17 + pgvector    │
                    │  FTS(jieba) + ANN + RRF 混合  │
                    │  任务队列 · 审计链 · 版本快照   │
                    └─────────────────────────────┘
```

- **后端**：Rust workspace 11 crates（`engram-api` / `engram-mcp` / `engram-core` / `engram-storage` / `engram-wiki-engine` / …）；领域表业务面 SQL 唯一收口在 `engram-storage::repo` 仓储层
- **前端**：React 19 + TypeScript + Vite 8 + Tailwind 4，十页 SPA，墨白双主题（Vercel/Geist 系设计语言）
- **单二进制单端口**：一个 `engram-server` 同源托管 API + SPA + MCP，本地/Docker 单机部署
- **中文友好**：jieba FTS + 向量混合检索（RRF 融合），零匹配时收紧向量阈值——返回空，不返回噪声

## 🔌 MCP 七工具工具面（渐进式发现）

engram-server 内置 MCP 服务端（官方 Rust SDK `rmcp`，Streamable HTTP）。
**工具面采用渐进式发现**：六个领域各一个入口工具 + 跨域全局检索
（AI 常驻上下文只占 7 个工具位），域内操作通过 action 按需发现——

- **L0 常驻目录**：每个域工具的描述自带「一行一操作」的紧凑目录，模型多数时候直接调对，零发现轮次
- **L1 按需手册**：`{"action":"help"}` 一轮取回全域操作的参数 JSON Schema
- **L2 错误自愈**：未知操作/坏参数的报错附带合法操作清单与 help 提示

```jsonc
// 调用形态：域工具 + action + 平铺参数
{"name": "todos", "arguments": {"action": "add", "title": "给记忆做个体检", "priority": "high"}}
```

| 域工具 | scope | 域内操作（action） |
|:--|:--|:--|
| 💬 `memory` | `memory` | `context`（冷启动上下文包）· `search` · `remember`（一句话记忆）· `write_session` · `append_session` · `list_sessions` · `get_session` · `list_atoms`（默认 active）· `entities` · `forget`（void/erase/**restore**；共 10） |
| 🧩 `projects` | `project` | `list` · `get` · `create` · `update` · `delete` · `batch_delete` · `types` · `location_add/update/delete` · `doc_add/get/search/update/delete` · **`doc_patch`**（行级补丁；共 16） |
| 🪄 `skills` | `skills` | `list` · `get` · `create` · `update` · `delete` · `import` · `file_get` · `file_put` · **`versions`/`restore`**（版本回滚）·（附属文件按路径读写，脚本由客户端本地执行；共 10） |
| 🕸️ `wiki` | `wiki` | `search`（片段化）· `list_pages` · `get_page` · `write_page` · `ingest` · `archive_query` · `graph` · `lint` · **`lint_deep`**（LLM 语义检查：矛盾/过时/缺页，产出入人审队列）· **`index`**（内容目录：按页型分组/入链数/摘要）· **`archive`**（问答产物归档为 analysis 页+双向链接）· `delete_page` · **`versions`/`version_content`/`restore_version`**（版本回滚与删页重建）· **`sources`/`delete_source`**（原料清理）· **`libraries`**（多库列表；全 action 可选 `library` 参数——真多库隔离；共 18） |
| ✅ `todos` | `todos` | 双形态：todo 行动项（open/done/archived）+ **ticket 工单**（severity/symptom/reproduce/acceptance/resolution + confirmed/in_progress/resolved/verified 状态机）· `add` · `list` · `get` · `done` · `update` · `delete`（共 6） |
| 🗺️ `codegraph` | `codegraph` | `list`（动态项目清单）· `register` · `index` · `sync`（走任务队列）· `query` · `delete`（共 6） |
| 🌐 `search_all` | 任一域 scope | 一次查询并发 memory/wiki/skills/todos/projects 各回 top-k 摘要（跨域一次查，精确检索仍用单域工具） |

按 key 的 scope 分权——AI 看到的工具面与它实际能调用的完全一致。
控制台「MCP」页 = 服务总开关（关闭即整体 503）+ 整域开关 + **域内单操作开关**
（停用的操作从目录与 help 手册隐身、调用被拒）；MCP 专用密钥在「设置 → API 密钥」签发。

Claude Code 三秒接入：

```bash
claude mcp add --transport http engram http://localhost:8080/mcp \
  --header "Authorization: Bearer amk_你的密钥"
```

> 🛡️ 远程部署需放开 Host 白名单：`AGENT_MEMORY_MCP_ALLOWED_HOSTS=mem.example.com`（默认仅 loopback，防 DNS rebinding）。

## 🚀 快速上手

### Docker（推荐）

```bash
git clone https://github.com/trtyr/engram && cd engram/deploy
cp .env.example .env      # 改掉全部 change-me 项（主密钥生成：openssl rand -hex 32）
docker compose up -d --build
# 控制台 → http://localhost:8080（端口/密码见 .env）
```

- **数据全在宿主**：PG 数据与应用文件 bind mount 到 `~/.engram/`（`postgres/`、`app/`、`backups/`），备份这一个目录即可
- 已有本地实例？`deploy/migrate-from-local.sh` 一键 pg_dump → 容器 restore（含关键表行数对照）
- 宿主跑 Clash TUN 类代理工具时容器直连出网会被污染：构建期传 `HTTP(S)_PROXY=http://host.docker.internal:<端口>` build args，运行时在 `.env` 配 `HTTPS_PROXY`（ OrbStack 还需检查 `~/.orbstack/vmconfig.json` 的 `network_proxy`）

### 本地开发

```bash
# 后端（需 PostgreSQL 17 + pgvector）
cd server
AGENT_MEMORY_DATABASE_URL=postgres://127.0.0.1:5432/engram \
AGENT_MEMORY_ADMIN_PASSWORD=admin123 \
AGENT_MEMORY_MASTER_KEY=$(python3 -c "print('ab'*32)") \
  cargo run --bin engram-server

# 前端（dev server，Vite 代理到 8080）
cd web && pnpm install && pnpm dev
```

## 📖 文档

**文档即数据**——全部沉淀在 engram 自身的 projects 域（本项目 `name=engram`），
按「总览 / 架构与实现 / 运维 / 产品 / 规划 / 决策 / 历史」组织，本地仓库不留文档
（仅保留本 README 作为 GitHub 门面）。

- 控制台：`/projects` → engram
- MCP：`projects` 工具的 `list` / `doc_search` / `doc_get`

## 🔁 数据迁移

```bash
# A 机导出迁移包（手动方式：文件拷到 B 机导入）
curl -H "Authorization: Bearer $ADMIN" http://a-host:8080/migrate/export -o transfer.json
# B 机导入（冲突跳过，合并语义可重复执行）
curl -X POST -H "Authorization: Bearer $ADMIN_B" -H "Content-Type: application/json"   --data-binary @transfer.json http://b-host:8080/migrate/import
# 或 B 机一键远程拉取（认证：A 机管理员密码，仅请求期使用不落库）
curl -X POST -H "Authorization: Bearer $ADMIN_B" -H "Content-Type: application/json"   -d '{"source_url":"http://a-host:8080","source_admin_password":"…"}' http://b-host:8080/migrate/pull
```

覆盖七域全部业务数据（用户记忆五表 + 实体关系 / 技能含附属文件 / Wiki 多库页面 / 项目记忆 / 待办）；
向量与 codegraph 索引为派生数据不迁移，导入端重建。控制台「设置 → 数据迁移」有同能力 UI。

## ✅ 门禁

```bash
cd server && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && pnpm run lint && pnpm exec tsc --noEmit && pnpm test && pnpm run build
```

---

<div align="center">

**Engram** —— 记忆，不该蒸发。🧠

<sub>MIT License · Built with Rust + React + PostgreSQL</sub>

</div>
