-- 0003: 系统域 — LLM 提供方与用量。
CREATE TABLE llm_providers (
    id               uuid PRIMARY KEY,
    name             text NOT NULL UNIQUE,
    base_url         text NOT NULL,
    -- AES-256-GCM: nonce(12B) || ciphertext || tag(16B)
    api_key_encrypted bytea NOT NULL,
    -- [{"id":"bge-m3","capabilities":["embedding"]}, ...]
    models           jsonb NOT NULL DEFAULT '[]',
    is_default       boolean NOT NULL DEFAULT false,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

-- 用量记账（每次 LLM 调用一行）
CREATE TABLE llm_usage (
    id             bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    provider       text NOT NULL,
    model          text NOT NULL,
    purpose        text NOT NULL,
    input_tokens   bigint NOT NULL DEFAULT 0,
    output_tokens  bigint NOT NULL DEFAULT 0,
    latency_ms     int     NOT NULL DEFAULT 0,
    job_id         uuid,
    ts             timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_llm_usage_ts ON llm_usage (ts DESC);
CREATE INDEX idx_llm_usage_purpose ON llm_usage (purpose, ts DESC);
