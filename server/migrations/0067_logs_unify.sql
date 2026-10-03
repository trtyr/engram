-- 0067: 日志统一（P010-T001）——logs 成为系统唯一时间线，job_events 并入。
--
-- 背景：此前「任务事件」另存 job_events 表，前端因此把「后台任务」当成与日志并列的
-- 第二种东西呈现，用户视角里出现两个概念。本迁移把任务事件并入 logs（带 job_id 字段），
-- 使「系统里发生的一切」都在同一条时间线上。
--
-- 纪律：迁移只增不改——job_events 表**保留**（留表停写，同 P008 的 wiki_review_items
-- 处理法），数据回填后代码不再写入；如需彻底移除，另开迁移。

-- 1) 任务事件的查询索引：logs.fields->>'job_id'（表达式索引，无需新增列）
CREATE INDEX IF NOT EXISTS idx_logs_job_id ON logs ((fields ->> 'job_id'))
    WHERE fields ? 'job_id';

-- 2) 历史回填：job_events → logs（ts 保序，id 自增随插入顺序）
--    target 记 'job.<kind>'，fields 带 job_id（原事件 data 平铺并入）
INSERT INTO logs (ts, level, target, message, fields)
SELECT
    je.ts,
    upper(je.level),
    'job.' || COALESCE(j.kind, 'unknown'),
    je.message,
    jsonb_build_object('job_id', je.job_id::text, 'backfilled', true)
        || COALESCE(je.data, '{}'::jsonb)
FROM job_events je
LEFT JOIN jobs j ON j.id = je.job_id
ORDER BY je.id;
