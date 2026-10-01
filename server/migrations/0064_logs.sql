-- 0064: 结构化日志落地（P005-T001）——tracing 事件的持久化面。
-- 设计：stdout JSON（docker logs 通道）之外的可查询存储；异步批量写入
-- （api/src/logging.rs），保留期 info 30 天 / debug 7 天由定时清理维护。
-- 红线：凭据值/token 永不入 fields（凭据纪律——写入口不放行 secrets 字段）。
CREATE TABLE logs (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ts          timestamptz NOT NULL DEFAULT now(),
    level       text NOT NULL,             -- TRACE/DEBUG/INFO/WARN/ERROR
    target      text NOT NULL,             -- 模块路径（tracing target）
    message     text NOT NULL,
    fields      jsonb NOT NULL DEFAULT '{}', -- 结构化字段（span/事件字段）
    request_id  text                       -- x-request-id 贯穿（P005-T002 起）
);

CREATE INDEX idx_logs_ts ON logs (ts DESC);
CREATE INDEX idx_logs_level_ts ON logs (level, ts DESC);
CREATE INDEX idx_logs_request_id ON logs (request_id) WHERE request_id IS NOT NULL;
