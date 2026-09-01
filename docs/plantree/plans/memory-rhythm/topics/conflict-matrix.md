# 冲突矩阵（AI × cron）

用户点名要考虑的极端情况。分两栏：**已内建**（多次事故烧出的既有防御，不重复建设）与**需要增量**。

## 已内建防御（复用，不新建）

| # | 冲突场景 | 既有防御 | 出处 |
|---|---|---|---|
| 1 | cron extract 认领会话后 AI 又 append | 已蒸馏会话 append 硬拒 400「请开新会话」 | append_session 约束 |
| 2 | 两个 AI 并发 append 同一会话 | 单语句 UPDATE...RETURNING 行级锁原子 | P12 修复 |
| 3 | cron 运行窗口内 AI 触发 deep purge | 两阶段 arm 5 分钟后悔药 + cancel | P-C |
| 4 | AI 与 cron 重复入队同任务 | 幂等键（extract-debounce-{秒桶} / deep-purge-arm-{秒桶}） | 防抖体系 |
| 5 | cron 中途 server 重启 | boot 对账 processing→pending + 死信重试 | P2 修复 |
| 6 | cron 全量整理撞上用户手动编辑 | 清退 > 钉住 > 自动重写 优先链 | 编辑能力 |
| 7 | cron 运行中 LLM provider 故障 | 重试/死信 + fail-loud（reembed 类） | jobs 体系 |
| 8 | cron 时钟与防抖桶错位 | 秒级桶（ae2d4a9 教训：分钟桶后悔药失灵） | arm 防抖秒级化 |

## 需要增量（R-2 范围）

| # | 场景 | 问题 | 候选方案 |
|---|---|---|---|
| A | cron full 蒸馏与 AI 触发蒸馏同时入队 | 幂等键不同（手动无桶），可能双跑 | full 也走幂等键（full-distill-{日桶}），同日二次入队返回既有 job |
| B | cron 维护窗口与 AI 高频写入交替 | extract 反复被 30s 防抖桶推迟， starvation | 防抖桶加最大推迟上限（如 5 分钟必跑） |
| C | cron 在静默时段外被误触发（时区/时钟漂移） | 凌晨跑打扰 / 白天没跑 | due 计算带时区 + 设置页静默时段 |
| D | 多 AI agent 同时活跃 + cron 同时跑 | 资源竞争（LLM 并发上限） | 队列并发上限（worker concurrency 已有 4，验证够用） |
| E | cron 记录性任务写入时 AI 正在 erase 同源会话 | 写 resurrect 已擦数据 | 引用完整性靠 source_refs 溯源 + erase 幂等（低概率，测试矩阵覆盖即可） |

## 测试矩阵（R-4 验收）

对上表 8+5 项各写一个可复现场景（一次性栈），重点：A（双入队）、B（starvation）、3（窗口 purge）。
