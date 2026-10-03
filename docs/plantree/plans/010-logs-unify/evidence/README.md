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

## 生产验证（HEAD=e2cb949，2026-10-03 07:05）

| 项 | 结果 |
|---|---|
| /ready | ready，**migration_version 67**（0067 已跑） |
| 容器 | engram-app-1 重建 healthy（部署前已备份 db.dump + env.bak） |
| ① job_events → logs 回填 | 回填行 1197 = job_events 总行 1197（**精确对应**） |
| ② job_id 索引 | `idx_logs_job_id` 存在 |
| ③ logs 带 job_id 行 | 1,253（含回填 1,197 + 部署后新增 56） |
| ④ **真实任务生命周期** | 触发 `POST /codegraph/projects/{id}/sync` → job `01a10094`，logs 中查到完整四步同一条时间线：`任务入队 → 开始执行 → 任务抢占 → 任务成功`（07:05:22–07:05:25） |
| ⑤ **MCP logs.query** | 按 job_id 返回 count=4（该任务全部行）；`logs.stats` 聚合真实计数（DEBUG 413,768 / INFO 86,835 / TRACE 75,095 / ERROR 24 / WARN 21，近 24h） |
| ⑥ 前端产物（30 分片） | 旧「后台任务」独立区块 = **0**；新「仅后台」范围选项 ≥1；「执行过程」≥1；job_id 关联 ≥1 |
| ⑦ job_events 表 | 留表停写（1197 行留存，代码不再写入） |
