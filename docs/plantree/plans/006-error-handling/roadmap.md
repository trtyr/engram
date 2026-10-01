# P006 Roadmap

> 任务状态唯一权威。痛点→缺口映射见 README。

## In Progress

（空——2026-10-02 五任务全落地）

## Next

（空——全部任务已结）

## Deferred

- **传播链剩余域批次**（T002 渐进下沉余量）：storage→llm/jobs→mcp/api 各域的 error enum → EngramError 桥接与 map_err 上下文补齐——随各域维护逐批落地（首批示范见 T002）。

## Done

- [x] 痛点映射 + 现状盘点（2026-10-01，见 README）。
- [x] **T001 · 统一错误模型**（d5d7d12）：core::errors（ErrorCategory 五类+EngramError+17 错误码注册表+登记纪律 runtime 强制）；retryable=显式声明可覆盖类别默认（强不变式 InternalBug 必不可重试）。
- [x] **T002 · 传播链首批**（619662c）：From<&WikiDocumentError> 桥 + 四失败点错误码化（SSRF-REJECTED/URL-FETCH-FAILED/PARSE-EMPTY/EMBED-DEGRADED）；其余域批次 Deferred。
- [x] **T003 · 反馈层**（91daebc）：PgLogLayer category=internal_bug 自动 alert=true + with_request_id 包装 + 集成测试全链路断言。
- [x] **T004 · 边界映射**（b387089）：ErrorEnvelope 增 category+request_id 字段 + 密钥痕迹零出现测试。
- [x] **T005 · 错误码全表**：engram projects《错误码全表》文档落库（doc_search 可检索）。
