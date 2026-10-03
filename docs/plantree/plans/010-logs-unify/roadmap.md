# P010 roadmap

## Done

- [x] **T001 数据层**：迁移 0067（logs 加 job_id 表达式索引 + job_events 历史回填；留表停写）✓ 31ad8a4
- [x] **T002 埋点**：emit 改写 logs（target=job.<kind>）；补「开始执行」起点——入队/开始/终态三跃迁全覆盖；events 读取改走 logs ✓ 31ad8a4
- [x] **T003 MCP 面**：新增 logs 域（query + stats）；dispatch 三处登记；storage 补 job_id 过滤与 count_logs；golden 快照更新 ✓ 31ad8a4
- [x] **T004 HTTP 面**：/logs 加 job_id 参数；/jobs/* 定名内部调度面；新增 job_id 生命周期测试 ✓ 31ad8a4
- [x] **T005 前端**：日志页重建为单一时间线（去独立任务区块，任务成为日志流中一类条目，可展开看过程/失败重跑，范围过滤，?scope=job 深链）✓ 31ad8a4
- [x] **T006 概念退役**：前端/MCP/HTTP 用户可见「任务」清零；修失效锚点与指向已删任务页的提示 ✓ 31ad8a4

## Done（续）

- [x] **T007 收官门禁 + 生产部署 + 生产实测**：四闸全绿（workspace EXIT=0/0 FAILED、clippy EXIT=0、web 四连）；门禁抓出并修正两轮真缺口（golden 快照过期 + mcp_test 硬编码断言）；生产部署至 e2cb949，迁移 66→67，实测 job_events 1197 行精确回填、真实任务生命周期四步同线可查、MCP logs 域返回真实数据 ✓ 2ad412d/891dbff/e2cb949
- [x] **T008 文档对齐**：engram projects 五篇增量（数据层/接口面/前端路由/mcp crate/api crate）；P010 注册；AGENTS.md 基线对齐

## Next

（空）

## Deferred

- `cg_projects.path` 仍存绝对路径（与宿主/容器形态耦合）——根治方向是存相对 id，读取时解析（P009 遗留）
- job_events 表物理删除（需另开迁移；当前留表停写无害）
