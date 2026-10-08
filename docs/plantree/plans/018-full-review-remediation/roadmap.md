# P018 · Roadmap

> 任务状态唯一权威。状态：`todo / doing / done / deferred`。来源 findings 的 file:line 见 evidence/。

## P0/P1 — 正确性

| ID | 任务 | 状态 | 证据 |
|---|---|---|---|
| T001 | 幂等键终态复用断链：`jobs/queue.rs:26-39` 幂等键预查无状态过滤——failed/dead/cancelled 同键任务永久阻塞新入队（注释与实现不符：声称「非终态或已成功才复用」）。修法：预查加 `WHERE status NOT IN ('failed','dead','cancelled')`（或等价终态放行），终态命中放行新插入。**必附回归测试** | done | 7520dae：ON CONFLICT DO UPDATE 按 revive 口径复位重跑（同 id 保事件历史）；回归 `idempotency_terminal_failure_allows_rerun` ✓ |
| T002 | 并发幂等键冲突处理：`jobs/queue.rs:28-64` 并发同键 INSERT 撞唯一约束抛 `Permanent("idempotency_conflict")`，调用方拿到报错而非既有任务——与幂等语义相悖。修法：`ON CONFLICT (idempotency_key) DO NOTHING` 后回查复用（或 duplicate 分支改查既有任务返回） | done | 7520dae：与非终态占位行冲突→回查复用；与终态行冲突→复位分支直接覆盖，不再抛 Permanent |
| T003 | rerank 重复索引校验：`core/search/unified.rs:311-323` 未校验 LLM 返回的 rerank 索引重复——重复索引使高分候选被静默降位。修法：解析时对已见索引去重/丢弃 | done | 7520dae：抽 `is_valid_order` 守卫（越界/重复/长度不符一律降级原序）+ 单测 |

## P3 — 正确性小修

| ID | 任务 | 状态 | 证据 |
|---|---|---|---|
| T004 | todos update 漏 title 长度校验：`core/todos.rs:300-306` update 未校验，而 create 同文件 `191-197` 有 200 字上限——复制粘贴改漏。补齐同款校验 | done | 71df416：与 create 同款 200 字上限校验 |
| T005 | 前端 BASE 三元残留：`web/src/lib/api.ts:6` `import.meta.env.DEV ? '' : ''` 两支恒等空串——简化为 `const API_BASE = ''`（P3 疑似：同源部署下无实际差异，纯可读性） | done | 71df416；web 四件套全绿 |

## 死代码清理（确定死，删除收益高/误杀风险低）

| ID | 任务 | 状态 | 证据 |
|---|---|---|---|
| T006 | ① `scripts/quality/split_domain_modules.py`、`split_engine_modules.py`、`split_mcp_lib.py`——wave-2 一次性治理残留；② `scripts/zztest_m10.py`——zz 前缀探针脚本，职责已由正式测试覆盖；③ `core/insights.rs` L319-327 `ts_now()`/`uuid7()` 两个 `#[allow(dead_code)]` 函数——删函数并连带删 allow 标注 | done | 71df416：四文件删除（-1520 行）+ 函数删除连带清 chrono 未用 import |

## 文档漂移（确证三处 + 一处疑似）

| ID | 任务 | 状态 | 证据 |
|---|---|---|---|
| T007 | ① `AGENTS.md:83` 迁移版本写 63，实际 **75**（migrations_test.rs:18 断言已核实）——修法可顺带在 migrations_test.rs 版本注释加「改此处须同步 AGENTS.md」防复发；② `README.md:131` 链接不存在的 `deploy/README.md`；③ `baseline/risk-hotspots.md:9` backup 兜底空卷条目已过时（`scripts/backup.sh` P001-T001 已改为报错退出，不再落回弃用 volume）；④ 疑似：`mcp/Cargo.toml:3` description 写「九域」vs `README.md:89-93`「十二域/13 工具位」——修前比对 DOMAIN_TOOLS 确证 | done | 71df416：①63→75+防复发提示✓ ③删过时行+弱口令挂 Q003✓ ④九域→十二域全清单✓；②**误报**——deploy/README.md 实存（ea68b9c 2026-09-27），codesleuth grep 结论错误，未改动 |

## logging 级别纪律（行为改动，小）

| ID | 任务 | 状态 | 证据 |
|---|---|---|---|
| T008 | 4xx 客户端错误降级：`api/src/error.rs:110-128` 所有 API 错误（含 BadRequest/NotFound/Conflict）统一打 ERROR——客户端噪音混入 ERROR 告警面。修法：4xx→WARN，5xx/internal_bug 保持 ERROR | done | 71df416：is_client_error() 分支 WARN/ERROR；web 四件套与 workspace test 全绿 |

## Deferred / 关联

- errors 路「传播链压平」（`api/error.rs:143-156` JobError/LlmError→Unavailable 丢语义；`mcp/guard.rs:5-40` mcp_err data 恒 None）→ P006 既有 Deferred（渐进下沉），本线不立项
- logging 路最弱①「secrets 过滤靠纪律非机制」→ open-questions Q007

## 计划外修复（第一批执行中发现）

| ID | 任务 | 状态 | 证据 |
|---|---|---|---|
| T009 | `core/tests/unified_test.rs` 两处 SQL bind 缺失（wiki_chunks `$3` / wiki_pages `$2` 漏绑，to_tsvector 参数）——HEAD 潜伏测试必炸，`e67a808`（P019 t6）引入时未跑全量。**教训：改 SQL 的提交必须过 workspace test 门** | done | 7520dae 一并修复；unified_test ✓ |
