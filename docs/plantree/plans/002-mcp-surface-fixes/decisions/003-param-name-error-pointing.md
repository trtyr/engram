# Decision 003 · EN-11 主键名不统一的修法：报错指路增强（方案 B）

- **日期**：2026-10-01
- **拍板人**：trtyr（ask_user_question 四选项拍板，选 B）
- **工单 / 任务**：EN-11 · P002-T005

## Context

十域主键参数名不统一（jobs/tickets 用 id、assets 用 asset_id、codegraph register 用 source_uri、location_add 用 asset 名字符串），调用方按相邻域直觉传名必踩 missing field 打回——每域首踩，持续性小磨损（三会话 5 实例）。工单 acceptance 给两条路：「统一命名」**或**「报错信息直接指路新名」。插入点已定位：dispatch.rs from_args 两段式解析（严格 → schema 宽容重试）。

## Decision

方案 **B · 报错指路增强**：`from_args` 严格解析失败时（value move 前）比对调用方键集与该操作 schema properties，missing field 场景报错点名「你传入的这些参数本操作不认识：[job_id]——疑似参数名不匹配（对照 action=help 改名重试）」——打回即学会，不做自动改名。

## 弃选

- **A · dispatch 别名兼容**（auto-rename job_id→id）：零打回，但按域别名表维护负担重 + 自动改名有掩盖真实错误的风险
- **C · 全量统一 schema 命名**：根治但破坏性（全部调用方/文档/golden 快照/测试连改），收益/风险比最差

## Consequences

- ✅ 改动一处（from_args 错误路径）全局生效；零 schema 变更——golden mcp_surface.json 字节级稳定
- ✅ 与 D7 受控宽容哲学一致：宽容解析可以，静默改语义不行（该哲学ADR D7 条目已同步增补）
- ⚠️ 仍打回一次（但报错即教学，比盲猜强）
- ⚠️ 生产实例需重新部署后生效

## 落地

commit `95a115d` · 回归 `mcp_param_hint_test` 2 用例 · `cargo test --workspace` EXIT=0（73 target ok，2026-10-01）。EN-11 resolved。
