-- 0010: 系统域 — 通用设置（键值，LLM 路由 / 触发参数等）。
CREATE TABLE settings (
    key        text PRIMARY KEY,
    value      jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
