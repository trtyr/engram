# P009 证据

## 本地门禁（HEAD=cb7bf7d）

| 闸 | 文件 | 结果 |
|---|---|---|
| workspace | `/tmp/gate_w1_ws_raw.log` | 见 WORKSPACE_EXIT |
| clippy 全仓 | `/tmp/gate_w1_clippy_raw.log` | CLIPPY_EXIT=0（04:06:37→04:07:12Z） |
| web 三连 | `/tmp/gate_w1_web.log` | TSC_EXIT=0 / LINT 0 error / Tests 89 passed / BUILD_EXIT=0 |

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
