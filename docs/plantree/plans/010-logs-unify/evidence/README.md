# P010 证据

## 修改前诊断（事实）

| 项 | 值 |
|---|---|
| logs 表行数（生产） | 484,644 |
| 其中带 job 字段的行 | 55 |
| 任务事件存放 | 独立表 job_events（约 24 种 job kind 在跑） |
| 前端呈现 | 日志页内含独立「后台任务」区块 → 用户视角出现两个概念 |

## 本地门禁（HEAD=31ad8a4）

| 闸 | 文件 | 结果 |
|---|---|---|
| workspace | `/tmp/gate_p010_ws.log` | 见 WORKSPACE_EXIT |
| clippy 全仓 | `/tmp/gate_p010_clippy.log` | 见 CLIPPY_EXIT |
| web 四连 | `/tmp/gate_p010_web.log` | TSC_EXIT=0 / LINT 0 error / Tests 91 passed / BUILD_EXIT=0 |
| 迁移测试 | — | migrations_test PASS（版本断言 66→67） |
| MCP 快照 | — | golden 更新后 PASS |
| logs 端点测试 | — | logging_test PASS（含新增 job_id 生命周期用例） |

## 前端产物验证（dist）

- 旧「后台任务」独立区块文案：**0 文件**
- 新「执行过程」：≥1 文件
- 新「仅后台」范围选项：≥1 文件

## 生产验证（部署后填）

- /ready 与迁移号
- logs 表中任务生命周期可查（job_id）
- MCP logs 域返回真实行
