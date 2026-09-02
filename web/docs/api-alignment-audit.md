# 前后端对接审计报告

> 审计日期：2026-08-28
> 范围：web 前端 vs server 后端
> 权威基准：后端 8090 OpenAPI
>
> ⚠️ **2026-09-02 过时标注**：knowledge 域端点已并入 /wiki 前缀（`/knowledge/*` 保留兼容别名），
> 前端融合成一个 Wiki 页（删 Knowledge.tsx）。本报告为历史快照，knowledge 行请按
> `/wiki/documents*` 理解；合并后的最新调用点分布见 [api.md](api.md)。

## 1. 端点覆盖核对

前端 44 个 `api.*` 调用点，逐一对照
`http://127.0.0.1:8090/openapi.json` 的 paths + method。

**结果：100% 覆盖，零缺漏。** 按域分布：

| 域 | 端点数 |
|---|---|
| auth | 1 |
| memory | 11 |
| knowledge | 7 |
| wiki | 18 |
| codegraph | 5 |
| jobs | 3 |
| settings/llm | 12 |
| search | 1 |

后端超前、前端未接的端点（保留在 API 层给 AI 客户端用）：

- `GET /memory/context`（AI 冷启动上下文包，非人读 UI）
- `GET /codegraph/projects/{id}`（列表已覆盖）
- `GET /memory/scenarios/{id}`（列表已覆盖）
- `GET /memory/sessions/{id}`、`GET /jobs/{id}`、
  `GET /knowledge/documents/{id}`（详情冗余）
- `GET /health`、`GET /ready`（健康检查，非 UI）

## 2. 字段类型核对

19 个手写类型（`src/lib/api.ts`）vs 生成 schema
（`src/lib/api-schema.ts`）。

### 发现 4 处不对齐（已修复）

| 类型 | 问题 | 修复 |
|---|---|---|
| `Document` | 缺 `updated_at`（schema required） | 补 `updated_at: string` |
| `CgProject` | 缺 `created_at`（schema required） | 补 `created_at: string` |
| `GraphDto.communities` | `top_slug`/`size` 标可选，后端已修违约 | 改为 required |
| `UsageRow` | 缺 `job_id`（schema 可选） | 补 `job_id?: string \| null` |

其余类型对齐。生成 schema 里用 `Record<string, never>`
占位的动态字段（如 `SessionDto.content`、`ScenarioDto.atom_refs`）
手写侧保留更精确的具体类型，属正常差异。

## 3. 错误处理契约核对

错误体契约 `{ error: { code, message, retryable } }`
与 `api.ts` 解析一致。

### 发现 1 处漂移（已修复）

`api.upload` 是独立实现，与 `req` 不一致：
写死 `code='upload_failed'`、不处理 401 清 token、不解析 `retryable`。
已对齐 `req`（401 清 token + 解析后端 `code`/`retryable`）。

其余：`req` 已正确处理 401 清 token、202/204 空响应（不误 parse）、
`{code,message,retryable}` 解包。

## 4. 后端问题

无。本次审计未发现后端行为与 spec 不符之处。

## 5. 验证

- `tsc -b`：零错
- `oxlint`：exit=0（零警告）
- `vitest run`：21 tests 全绿
