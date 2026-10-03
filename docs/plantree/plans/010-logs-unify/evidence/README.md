# P010 证据

## 修改前诊断（事实）

| 项 | 值 |
|---|---|
| logs 表行数（生产） | 484,644 |
| 其中带 job 字段的行 | 55 |
| 任务事件存放 | 独立表 job_events（约 24 种 job kind 在跑） |
| 前端呈现 | 日志页内含独立「后台任务」区块 → 用户视角出现两个概念 |

## 本地门禁（终态 HEAD=891dbff）

| 闸 | 文件 | 结果 |
|---|---|---|
| workspace | `/tmp/gate_p010_ws3.log` | **WORKSPACE_EXIT=0，0 FAILED**（891dbff，06:18:42→06:44:43Z） |
| clippy 全仓 | `/tmp/gate_p010_clippy3.log` | **CLIPPY_EXIT=0**（891dbff） |
| web 四连 | `/tmp/gate_p010_web.log` | TSC_EXIT=0 / LINT 0 error / Tests 91 passed / BUILD_EXIT=0 |
| 迁移测试 | — | migrations_test PASS（版本断言 66→67） |
| MCP 快照 | `mcp_surface.json` | 13 工具位（+logs）；UPDATE_GOLDEN 后 PASS |
| logs 端点测试 | — | logging_test PASS（含 job_id 生命周期用例） |
| mcp_test | — | 17/17 PASS（三处硬编码断言同步 logs 域） |

### 门禁抓出的真实缺口（两轮修正，都是真问题不是抖动）

| 轮次 | 失败 | 根因 | 修正 |
|---|---|---|---|
| 首轮 | `mcp_tool_surface_is_byte_stable` | t6 改描述文案后 golden 快照过期（UPDATE_GOLDEN 在文案改动前跑的） | 2ad412d 重生成 + 修 jobs 域描述悬空片段 |
| 次轮 | `mcp_test` 3 断言 | 新增 logs 域打破硬编码清单（可见面/工具总数 12→13/开关后保留域） | 891dbff 三处同步 |
| 首轮 | 同上 | HEAD stamp 与提交时序 | 每轮在干净新 HEAD 上重跑（ws/clippy 并行发车） |

## 前端产物验证（dist）

- 旧「后台任务」独立区块文案：**0 文件**
- 新「执行过程」：≥1 文件
- 新「仅后台」范围选项：≥1 文件

## 生产验证（部署后填）

- /ready 与迁移号
- logs 表中任务生命周期可查（job_id）
- MCP logs 域返回真实行
