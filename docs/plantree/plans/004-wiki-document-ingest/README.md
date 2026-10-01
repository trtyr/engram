# P004 · wiki 原文导入链路修复（EN-32/31 + URL 提取增强）

## Scope

EN-32（P1）嵌入熔断滞留故障修复 + EN-31（P2）导入降级设计拍板 + URL 正文提取增强方向评估。
Affected Modules: `server/core/src/wiki_docs/`（pipeline/ssrf）、`server/crates/mcp/src/wiki*`（如工具面补齐）、`server/crates/parsing/`（如提取增强）。

## 破案结论（2026-10-01 代码实证）

链路**存在且完整**：`document_add(url)` → `IngestSource::Url` → `enqueue_ingest`（sha 幂等）→ `parse_job`（SSRF safe_fetch 抓取 + parse_bytes 正文提取）→ `chunk_job`（chunk_text 分块）→ `embed_job`（批量补嵌 ≤64）→ set_ready → 自动织入 wiki。用户感知「没有提取体系」实为**链路断在嵌入环节，从外看像没干活**。

**真实断点两处**：

1. **embed 链路缺「重试耗尽落终态」守卫**：fetch 路径有 W-1/W-2 守卫（`steps.rs:289-319`——Retryable 最后一试也 mark_failed，注释明确防「job dead + 文档 pending 孤儿态」）；**embed 路径无对等守卫**（`steps.rs:270-275`——熔断打开 = `LlmError::Transient` → `JobError::Retryable` 上抛，重试耗尽 job dead 后无人更新文档状态）→ 文档永久滞留 `embedding` + `error=null`（三篇实况：14:31/14:37 入库，22:00+ 仍零进展）。
2. **无自动续跑**：K1 卡死自愈（`steps.rs:171-190`）只在「同 sha 重新 document_add」的幂等命中路径触发；没人重提交就永不触发。熔断恢复后无任何机制重入队 dead 的 embed job。

**已有恢复入口**：HTTP `POST /wiki/documents/{id}/re-embed`（`wiki_docs_api.rs:226`，202 入队，只补 embed_failed/NULL 向量块）——但 **MCP 工具面无此 action**，agent 侧无直达恢复手段。

## 转向（2026-10-01 用户拍板）

用户对架构定位的批评（原话要点）：「这就是一个向量数据库，不是 wiki」——engram wiki 的维护权在后端自动流水线（一次性 LLM 织入 + 向量检索），agent 无法像 Karpathy LLM Wiki 模式（gist 442a6bf）那样**作为维护者**操作。**拍板：wiki 所有能力经 MCP 开放，由外部 Agent 维护**（Karpathy 模式本意）。落地 = T007（工具面全量对齐，主体）+ T008（agent 工作流）。后端自动织入的去留见 Q005。

## File Map

- `roadmap.md` — 任务状态唯一权威
- `open-questions.md` — Q001 恢复机制选型 / Q002 提取增强选型
- `evidence/` — （待补：修复 PR 的测试证据）

## 关联

- tickets：EN-32（P1 本计划主体）、EN-31（P2 设计拍板，Q001/Q003 承接）、EN-33（P3 study 域，独立另议不入本计划）
- 架构文档：engram projects《构建块 · wiki-engine 与 search》`01a0e6f8-134c`
- P005（日志）/ P006（错误处理）：harness 的工具审计与失败归因消费两线基础设施
- **LLM 测试通道**（2026-10-01 用户提供）：网关 `https://newapi.trtyr.top`（通用，OpenAI 兼容，tool calling 已确认支持）+ 测试专用 key 存 credentials `newapi.trtyr.top/api-key-engram-test`（值永不入文档/代码，取用走 credentials get）——harness 本地测试与 engram 开发测试复用
