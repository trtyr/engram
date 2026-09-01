# D-002 cron 宿主与语义层（2026-09-01）

**决策**：cron 住**外部**（系统 crontab 打 API）+ 语义取**层 A + 层 B**（维护性 full 蒸馏 + 记录性兜底：pending 会话超时由 cron 扫走）。

**推翻推荐**：goal 问卷两题均选非推荐项（原推荐「server 内置 scheduler」+「只做维护性」）。用户理由：外部 cron 灵活；记录性兜底多一层不依赖 AI 自觉的保险（北极星「真记性不需要主人提醒自己在记」）。

**后果**：

- server 侧不做内置 scheduler——jobs 表 due 轮询留作已有任务队列，不新增周期注册器
- cron 触发走既有端点 + `via:"cron"` 标记：consolidate 日桶幂等（防 retry 风暴/双行 crontab），extract 永不去重（扫 pending 是兜底本意）
- 心跳（`POST /memory/rhythm/heartbeat`）让 server 观察外部 cron 的存活，设置页据此判逾期
- **静默时段 = crontab 自控**（open-questions #4 随外部宿主消解——server 不控制运行时间）
- 观察面（设置页）：心跳监控 + 积压年龄 + 安装向导 + 节律事件流，全部复用 jobs 表

**反向约束**：外部宿主下 server 无法替用户「关掉 cron」——设置页的观察面不提供开关，只提供逾期警示与安装向导；真正的启停由用户改 crontab。
