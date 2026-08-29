# 当前状态（验证基线 2026-08-29）

> `project-init` 全栈归档（08-28）+ 开放项修复轮（08-29）后的真实验证基线。工作区 clean。
> 08-28 首轮基线发现的 7 条开放项已全部处置（见「开放项处置记录」）。

## 环境

| 项 | 值 |
|---|---|
| rustc / cargo | 1.97.1（本机） |
| pnpm | 11.20.0（packageManager 字段钉住） |
| PostgreSQL（本机，集成测试） | 16.14（Homebrew，trust 认证，127.0.0.1:5432） |
| Docker | 29.4.0（OrbStack） |
| Playwright chromium | 本地缓存可用 |

## 验证命令与结果（2026-08-29，修复后）

### 后端（server/）

| 命令 | exit | 结果 |
|---|---|---|
| `cargo fmt --check` | 0 | ✅（修复 wiki_test.rs 漂移：cargo fmt 全量格式化） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | ✅（修复 distill/extract.rs collapsible-if+死赋值、wiki_test 未用形参/死变量、llm_api 冗余闭包） |
| `cargo test --workspace` | 0 | ✅ **100 passed / 0 failed**（本机 PG，约 3.5 分钟） |
| `cargo run -q -p agent-memory-api --bin openapi-dump` | 0 | ✅ 55 paths |

### 前端（web/）

| 命令 | exit | 结果 |
|---|---|---|
| `pnpm install --frozen-lockfile` | 0 | ✅ |
| `pnpm run lint`（oxlint） | 0 | ✅ |
| `pnpm test`（vitest） | 0 | ✅ 4 文件 **21 测试全过** |
| `pnpm run build`（tsc -b + vite build） | 0 | ✅ 无 >500kB 警告；路由级 lazy 后初始 bundle 612→**280kB**，Wiki 页 294kB 按需 |

### 契约 / e2e / Docker

| 检查 | 结果 |
|---|---|
| api-schema 漂移（dump→再生成→git diff） | ✅ 零漂移 |
| Playwright 全旅程（本地无 provider 栈，原生 cargo run :19180 + 本机 PG） | ✅ **PASS 1/0（22s）**——LLM 依赖段自动跳过（note 注解），登录/上传/会话/CodeGraph 索引→查询/Jobs 全过 |
| `docker build --target web-build` | ✅ exit 0（corepack+pnpm 路径通）；完整镜像构建按用户决定跳过（开发阶段），由 CI `docker` job 验证 |

## 开放项处置记录（对应 08-28 基线的 7 条）

1. **pnpm 迁移善后** ✅：ci.yml（web/api-types）、e2e.yml 全部改 pnpm/action-setup@v4 + `pnpm install --frozen-lockfile`；deploy/Dockerfile Stage1 改 corepack（读 packageManager 字段）+ pnpm。`.github/` 与 Dockerfile 零 `npm ci`/`package-lock` 残留。
2. **fmt/clippy** ✅：见上表（4 处代码修复 + 全量格式化）。
3. **CI backend 缺 PG** ✅：backend job 加 `pgvector/pgvector:pg17` service（5432:5432，pg_isready 健康检查）+ `AM_TEST_PG_URL`。
4. **e2e 蒸馏断言** ✅：journey.spec.ts 探测 `GET /settings/llm/providers`，无 provider 时跳过蒸馏/Wiki 生成断言（annotation 留痕）；另修复 4 处 UI 重设计导致的选择器漂移（getByLabel 登录、会话/原子/图谱中文 tab、rounded-xl 卡片）。
5. **cargo audit** ✅：见下方「依赖审计」。
6. **mermaid 大 chunk** ✅：七域页 React.lazy 路由级分割；mermaid 主入口（~662kB，发布产物固有体积，仅渲染图时加载）→ `chunkSizeWarningLimit: 800` + 注释说明。
7. **server/docs 过时** ✅：迁移 12→14、路由 8→9、testcontainers→本机 PG、新端点（provider 生命周期/re-encrypt/re-embed）已就地修正；server/docs/current-state.md 加横幅指向本文档。

## 依赖审计（cargo audit，2026-08-29 复扫）

- **1 漏洞**：`rsa 0.9.10` — RUSTSEC-2023-0071（Marvin Attack 时序侧信道，medium 5.9，上游无修复版）。
  **决策：接受（无需修复）**——`cargo tree -i rsa --target all` 零反向依赖：rsa 是 lockfile 孤儿条目（testcontainers 时代遗物，随 f9491ed 移除依赖后残留于 lock），不参与任何编译产物，无可达攻击面。彻底清除需重建 Cargo.lock（418 依赖全量重解析，版本漂移风险大于收益）。
- **4 allowed warnings**：`fxhash`（unmaintained，RUSTSEC-2025-0057）、`rand_os`（unmaintained，RUSTSEC-2025-0124）、`ttf-parser`（unmaintained，RUSTSEC-2026-0192）、`chacha20`（yanked）——均为传递依赖且非高危，维持观察。
- 对比 08-26 旧基线：`tokio-tar` 漏洞已随 testcontainers 移除消失 ✓。

## CI 现状（2026-08-29 收官）

origin/main @ `0d1d145`，两个 workflow 最新 run **全绿**（gh 实查）：

| workflow | run | 结果 |
|---|---|---|
| CI | 33263092482（6m32s） | ✅ backend(fmt/clippy/test×100) + web(pnpm 全链) + api-types(零漂移) + docker(镜像构建) |
| e2e | 33263092491（10m57s） | ✅ compose 栈 Playwright 全旅程（无 provider 跳过 LLM 段） |

修复过程两轮：第一轮 CI 后端挂在 **rust-embed 编译期找不到 `web/dist`**（runner 无前端产物；phase-6 起就存在的暗坑，此前一直被 fmt 失败掩盖）——backend job 补 `mkdir -p ../web/dist` 占位后全绿。另：CI stable 工具链已到 1.98.0（本地 1.97.1→1.98.0 升级后 clippy 复验通过，两版均无 lint）。

提交序列（本次修复轮，共 6 个）：`d1884f4` fix(server) fmt/clippy → `a6005ae` fix(ci) pnpm+PG service → `38d0aaf` fix(e2e) 条件跳过+选择器 → `53f7e45` perf(web) 路由级 lazy → `c21f7a0` docs 全栈归档 → `0d1d145` fix(ci) web/dist 占位。
