# P013 · 评审链路收编（待审通道移除 + JEV 生产部署）

> 2026-10-04 用户拍板三点之一/之二。状态：**planning**（拍板意图已定，执行待排期）

## Scope

两个主题：**人审待审通道整体移除** + **生产 JEV 决策模型补配**。

### 主题 A：待审（needs_review）通道整体移除

用户原话：「前端有个待审的东西，我们不需要。你看看后端，如果有的话，给它清理掉。前端也清理掉。」

已侦察的影响面（2026-10-04 grep）：

| 层 | 位置 | 内容 |
|---|---|---|
| core | memory/atoms.rs | needs_review 字段、confidence<0.55 自动留审、pending_review_atoms/review_confirm/review_discard |
| core | memory/search.rs | context_pack 待审代问（5 条顺口确认） |
| api | routes/memory_api/atoms.rs | 待审原子查询/确认/丢弃路由 |
| mcp | memory.rs + memory_write.rs + dispatch.rs | review(mode=result/confirm/discard) 三模式 |
| web | Memory.tsx + lib/api.ts + 组件测试 | 待审 UI + 审核操作 |
| 测试 | api/mcp/web 各测试面 | 待审断言清理 |

**范围红线**：
- **study 域 needs_review 是 SRS 复习标记，不同概念，严禁误伤**（core/study.rs、mcp/study.rs、api/study_api.rs 不在删除范围）
- JEV 级联（P011 T014）的 Review band 依赖 needs_review 落点——同链联动重新定义

### 主题 B：生产 JEV 决策模型部署

用户原话：「你生产时忘记了去部署 JEV 模型。」

现状（2026-10-04 生产侦察）：llm_providers 只有 `New API/MiniMax-M3(chat)` + `向量模型/Qwen3-Embedding-8B(embedding)` 两行——**JEV 专用配置区块缺失**，重蒸链上 JEV 级联走了什么 fallback 待查。

待办：

- [ ] 查 T013 JEV resolve 机制（决策模型路由到哪行 provider、缺失时行为）
- [ ] 生产 llm_providers 补 JEV 配置（凭证引用 credentials 域 `openrouter/engram`，只引名称不引值）
- [ ] 验证重蒸产物：JEV 级联在生产实际生效（logs/llm_usage 留痕）

## Affected Modules

server/core/memory, server/api, server/mcp, web, deploy（生产配置）

## 拍板点（执行前必须过）

1. **confidence<0.55 原子去向**：待审通道删除后，低置信原子是「直接生效」还是「不收」（宁缺毋滥）？
2. **JEV Review band 去向**：三档路由的中档（原留 needs_review）改为直接生效还是归入拒绝？
3. 存量已留审原子（生产现量待查）的处置：批量 confirm / 批量 discard / 随迁移清理？

## Links

- JEV 设计与级联：P011（memory-audit-debts）T013/T014 + engram 档案蒸馏链篇
- 生产 LLM 配置侦察：2026-10-04 会话记录