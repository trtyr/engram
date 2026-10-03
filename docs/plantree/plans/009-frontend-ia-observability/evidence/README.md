# P009 证据

## 审计首轮驳回与修正

| 驳回点 | 修正 |
|---|---|
| 声称「/jobs 路由零残留」不实：Dashboard.tsx:133（近期活动行）与 :314（失败 N 徽章）仍硬编码 `to="/jobs"`，点入白屏 | c0875eb 两处改 `/logs#jobs`；Logs.tsx 任务区块加 `id="jobs"` + hash effect 平滑滚动 |
| （根治）此类漏网人眼扫不住 | c0875eb 新增 `src/routes-consistency.test.ts`：扫全部源码 `<Link to="/...">` 与 `NAV_GROUPS` 的 `to:'/...'`，断言命中 App.tsx 已声明 Route（支持 `:param`）；突变验证塞坏链接即 FAIL |

## 本地门禁（HEAD=cb7bf7d → 修正后 c0875eb）

| 闸 | 文件 | 结果 |
|---|---|---|
| workspace | `/tmp/gate_w1_ws_raw.log`（w1, HEAD=cb7bf7d）WORKSPACE_EXIT=0, 0 FAILED；`/tmp/gate_w2_ws_raw.log`（w2, HEAD=c0875eb）见其 EXIT |
| clippy 全仓 | `/tmp/gate_w1_clippy_raw.log` CLIPPY_EXIT=0；c0875eb 后复跑 0 error |
| web 三连 | `/tmp/gate_w1_web.log`（89 测试）；`/tmp/gate_w2_web.log`（HEAD=c0875eb，**91 测试**含新增路由守护）TSC/LINT/BUILD 全 0 |

## 生产验证（codegraph 修复）

- 修复前：14+ 项目全部 `项目路径不存在（/root/.engram/app/codegraph/...）`
- 根因：compose 未设 `AGENT_MEMORY_DATA_DIR`，容器内 HOME fallback `/root/.engram/app`（可写层）≠ 卷挂载点 `/app/data`
- 修复后：
  - `docker exec engram-app-1 sh -c 'echo $AGENT_MEMORY_DATA_DIR'` → `/app/data`
  - DB `cg_projects.path` 20 条全部迁移到 `/app/data/codegraph/...`
  - 4 个 cloud_index 项目重新 register（clone+index）：**全部 status=ready**
  - **实测 `POST /codegraph/projects/{id}/query {kind:explore,target:dispatch}` → 返回 32 符号/4 文件**；`{kind:search,target:tool_router}` → 返回真实节点（含 filePath/行号）——非 path 报错

## 未恢复项

16 个 `client_upload` 型条目：产物仅存客户端本机，服务端无备份，容器重建后不可恢复——需客户端重新 index+upload。
