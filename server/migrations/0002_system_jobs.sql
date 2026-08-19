-- 0002: 系统域 — 任务队列表。
-- 状态机: pending -> running -> succeeded | failed(可重试->pending) | dead(重试耗尽)
CREATE TABLE jobs (
    id                  uuid PRIMARY KEY,
    kind                text NOT NULL,
    payload             jsonb NOT NULL DEFAULT '{}',
    status              text NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','running','succeeded','failed','dead')),
    attempts            int  NOT NULL DEFAULT 0,
    max_attempts        int  NOT NULL DEFAULT 3,
    idempotency_key     text UNIQUE,
    error               text,
    progress            jsonb,
    created_at          timestamptz NOT NULL DEFAULT now(),
    started_at          timestamptz,
    finished_at         timestamptz,
    due_at              timestamptz NOT NULL DEFAULT now(),
    locked_by           text,
    locked_at           timestamptz,
    visibility_timeout_s int  NOT NULL DEFAULT 300
);

-- 抢任务扫描路径：待处理且到期
CREATE INDEX idx_jobs_claimable ON jobs (due_at) WHERE status = 'pending';
CREATE INDEX idx_jobs_status_kind ON jobs (status, kind);
CREATE INDEX idx_jobs_visibility ON jobs (locked_at) WHERE status = 'running';

-- 任务事件流（追加）
CREATE TABLE job_events (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    job_id      uuid NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    ts          timestamptz NOT NULL DEFAULT now(),
    level       text NOT NULL DEFAULT 'info' CHECK (level IN ('info','warn','error')),
    message     text NOT NULL,
    data        jsonb
);

CREATE INDEX idx_job_events_job ON job_events (job_id, id);
