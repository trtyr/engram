-- 0019 deep purge 后悔药（P-C 两阶段，2026-08-31）：取消态进合法状态集。
-- armed job 的 cancel 写 status='cancelled'——没有它，后悔药端点 500。
ALTER TABLE jobs DROP CONSTRAINT jobs_status_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_status_check
    CHECK (status = ANY (ARRAY['pending'::text, 'running'::text, 'succeeded'::text, 'failed'::text, 'dead'::text, 'cancelled'::text]));
