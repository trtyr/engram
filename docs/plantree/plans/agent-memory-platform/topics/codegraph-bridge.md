# Topic — CodeGraph 桥

复用 colbymchenry/codegraph（D0005），平台只做包装：注册、同步、代理查询、错误归一。**不解析任何代码**。

## 集成方式

- 子进程调用 codegraph CLI，全部走 `--json` 输出
- 版本 pin：安装时锁定版本（Q3），启动时探测版本不匹配 → cg_projects 全部标 `version_mismatch` 并告警，不盲跑
- 超时：init/index 10min、sync 60s、query 30s；超时 kill 子进程并归一为 timeout 错误
- 工作目录：`data/codegraph/<project_id>/`（项目源码 clone/挂载于此，`.codegraph/` 由上游自管）

## 项目生命周期

```text
registered → indexing（codegraph init/index）→ ready
                  ↘ error                     ↘ sync 中 → ready
```

- 注册：本地卷路径（校验存在）或 git URL（平台负责 clone 到工作目录，深度1）
- 同步：`POST :id/sync` → codegraph sync（增量）；ready 后查询前自动做 staleness 检查（CLI status）
- stats（symbols/edges/files 计数）从 `codegraph status --json` 解析，缓存于表，同步后刷新

## 查询代理

`POST /codegraph/query`：

| kind | CLI 映射 |
|---|---|
| explore | `codegraph explore <q> --json` |
| node | `codegraph node <sym>` |
| search | `codegraph query <s> --json` |
| callers / callees | 同名子命令 `--json` |
| impact | `codegraph impact <sym> --json --depth N` |

- 归一化：上游 JSON → 稳定 DTO；上游字段变更 → 解析层单点适配（防 R3 扩散）
- 大输出截断保护：响应超 budget 上限时摘要化 + 提示用更精确查询

## Docker 内运行

镜像安装 codegraph（自带 Node runtime，不依赖系统 Node）；`data/codegraph` 为卷。容器内无 watch 场景 → 不依赖上游 daemon，全部按需 sync。
