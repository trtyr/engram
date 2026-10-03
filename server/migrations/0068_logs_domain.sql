-- 0068: 日志域化（P011-T019）——logs 加 domain 列，每功能全量日志按域聚合。
--
-- 背景：P010 后 logs 是系统唯一时间线，但「这是哪个功能域的日志」只能靠 target
-- 前缀猜（job.<kind> / auth.login / audit.*）。域化后：memory / wiki / codegraph /
-- system 四域显式标注，读取面（MCP/HTTP/前端）可按域过滤。
--
-- 纪律：迁移只增不改；历史行按 target 前缀一次性回填（推断不出 = system）。

-- 1) 域列：默认 system（非 job / 未识别前缀的安全值）
ALTER TABLE logs ADD COLUMN IF NOT EXISTS domain text NOT NULL DEFAULT 'system';

-- 2) 按域过滤 + 时间排序的组合索引（读取面主查询模式）
CREATE INDEX IF NOT EXISTS idx_logs_domain_ts ON logs (domain, ts DESC);

-- 3) 历史回填：job.<kind> 按 kind 前缀推断域（与 jobs::domain_for_kind 同表驱动）
UPDATE logs SET domain = CASE
    WHEN target LIKE 'job.extract_atoms%' OR target LIKE 'job.arbitrate_atoms%'
      OR target LIKE 'job.organize_scenarios%' OR target LIKE 'job.distill_persona%'
      OR target LIKE 'job.consolidate%' OR target LIKE 'job.reembed_memory%'
      OR target LIKE 'job.deep_purge%' OR target LIKE 'audit.%' THEN 'memory'
    WHEN target LIKE 'job.wiki_%' THEN 'wiki'
    WHEN target LIKE 'job.cg_%' THEN 'codegraph'
    ELSE 'system'
END
WHERE domain = 'system' AND target LIKE 'job.%';
