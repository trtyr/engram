# 当前状态（验证基线）

> ⚠️ 本文档是 2026-08-26 的 backend-only 基线，部分事实已过时（testcontainers→本机 PG、12→14 迁移、56→100 tests、新增端点）。
> **最新全栈验证基线见仓库根 [docs/current-state.md](../../docs/current-state.md)（2026-08-29）**；本文档保留作历史参考，架构/数据模型/API 等事实性条目已在各文档就地修正。

> 本文档记录 `project-init` 在 2026-08-26 对 `server/` 后端做的真实验证结果。所有命令在 `server/` 下运行。

## 环境

| 项 | 值 |
|---|---|
| rustc / cargo | 1.97.1 |
| Docker | 29.4.0（testcontainers 可用） |
| 构建缓存 | `target/debug` 已有缓存（1638 个 .d） |

## 验证命令与结果

| 命令 | exit code | 结果 |
|---|---|---|
| `cargo fmt --check` | 0 | ✅ 无格式问题 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | ✅ 无 warning（1.25s） |
| `cargo test --workspace` | 0 | ✅ **56 passed, 0 failed**（约 2.5–3 分钟，含 testcontainers 集成测试） |
| `cargo build`（隐含于 clippy/test） | — | ✅ 编译通过 |

### 测试明细

| crate | 单元测试 | 集成测试（testcontainers） |
|---|---|---|
| api | 0 | auth_test 2 |
| cg-bridge | 3 | cg_test 3 |
| core | 4 | knowledge_test 4 |
| distill | 0 | distill_test 3 |
| jobs | 0 | job_lifecycle_test 5 |
| llm | 4 | provider_test 3 |
| parsing | 4 | 0 |
| search | 4 | search_test 1 |
| storage | 0 | migrations_test 2 |
| wiki-engine | 11 | wiki_test 3 |
| **合计** | **30** | **26** |

总计 **56 passed / 0 failed**。

## 未提交更改（工作区）

- **45 个文件被删除（未暂存，` D`）**：根目录 `README.md`、`AGENTS.md`、`CHANGELOG.md`、`docs/AI-INTERFACE.md`，以及整个 `docs/plantree/`（baseline、plans、evidence、decisions、ideas、phases 等全套规划树）。
- 这些文件在 git HEAD 中仍存在（可 `git restore` 恢复）。
- 本次按用户决定：**忽略删除、不恢复**；Recon 时从 `git show HEAD:...` 读取了旧 README 与 AGENTS 作参考。
- 本次新增 7 个文档（`docs/*.md`，untracked `??`），未提交。

## 开放项 / 已知问题

1. **cargo audit 未运行**（不在 project-init verify 清单内）。旧 `AGENTS.md`（2026-08-25 复扫）记录：2 个漏洞（`tokio-tar` 仅 testcontainers dev 依赖、`rsa` 为 lockfile 孤儿），4 个 unmaintained 警告（`ttf-parser`、`fxhash`、`rand_os`、`rustls-pemfile`——后三个为 advisory-db 更新新出现，非代码回归）。**建议后续跑一次 `cargo audit` 复核**。
2. **`pdf-extract` 的 lopdf 高危（原 Q8）已解决**：`pdf-extract 0.12.0` 内部 lopdf ≥0.42。
3. **前端（`web/`）未文档化**：本次 scope 为 backend-only，web/ 的结构、技术栈、e2e 未覆盖。
4. **被删的 `docs/plantree/` 是原权威规划树**：其内容（路线图、决策 D0005–D0011、证据、phase 0–7）在删除后仅存于 git 历史。本次 archive 是独立重建，不依赖它。

## 本次验证的局限

- 未起 Docker compose 栈做真实 HTTP 冒烟（`/health` `/ready`），只跑了编译/测试/静态检查。
- 未跑 `scripts/verify-*.sh` 三套端到端验证脚本。
- 未跑前端相关门禁（tsc/vitest/playwright）。
