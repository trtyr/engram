# Phase 7 — 交付打磨与发布

**目标**：从「功能全有」到「别人拿到就能跑」：生产化 compose、e2e 全旅程、备份、文档、版本发布。

## 前置

Phase 6。

## 交付物

### 生产化交付

- [ ] Dockerfile 终版：多阶段（前端 build → cargo build → 运行时含 codegraph），镜像体积审视（记录最终大小于 evidence）
- [ ] compose 生产化：restart 策略、healthcheck、卷（pg data / uploads / wiki-sources / codegraph）、资源限制建议注释
- [ ] `deploy/backup.sh`：pg_dump + data 卷打包；`restore` 验证脚本（备份可恢复才算数）
- [ ] 首次启动引导：无 provider 时 UI 引导配置（否则蒸馏任务失败提示明确）

### E2E 全量

- [ ] playwright 对 compose 栈跑全旅程套件：登录→配置 provider→各域写入→蒸馏→检索→人工纠偏→版本回滚
- [ ] AI 视角 e2e：纯 HTTP 脚本模拟 AI 客户端完成「写会话→取 context→搜知识→查图谱」闭环（这是 D0001 的验收本体）

### 文档

- [ ] `README.md`（人类）：架构图、快速启动、配置说明、备份恢复
- [ ] `docs/AI-INTERFACE.md`（AI 客户端）：API 速览 + 认证 + 典型工作流 + 预算约定——写成可直接粘贴进任意 agent 系统提示词的「工具说明书」（D0004 的弥补层）
- [ ] CHANGELOG.md + 版本 tag（v0.1.0）

## 出口标准

1. 干净机器（或干净目录）clone → `cp .env.example .env`（填密钥）→ `docker compose up -d` → e2e 全绿——全程无手工干预
2. 备份→销毁→恢复→数据完整（atoms/wiki 页/codegraph 注册全在）
3. AI-INTERFACE.md 交给一个新 agent 会话，仅凭文档完成上述 AI 视角闭环（真实验证，evidence 留档）
4. 全部 CI 门（含 e2e job）绿；roadmap 全部 phase 标 Done 并链接 evidence

## 关联

- 决策：D0004、D0008
- 门禁：[baseline/test-and-release-gates](../../../baseline/test-and-release-gates.md)
