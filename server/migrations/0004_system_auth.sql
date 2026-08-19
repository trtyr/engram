-- 0004: 系统域 — 鉴权（API key + 管理员会话，见 D0007/D0011）。
CREATE TABLE api_keys (
    id           uuid PRIMARY KEY,
    name         text NOT NULL,
    key_hash     text NOT NULL UNIQUE,     -- sha256(raw key)
    key_prefix   text NOT NULL,            -- 展示用前 8 字符
    scopes       jsonb NOT NULL DEFAULT '["memory","knowledge","wiki","codegraph"]',
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    revoked_at   timestamptz
);

CREATE TABLE admin_sessions (
    token_hash   text PRIMARY KEY,          -- sha256(opaque token)
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL,
    last_used_at timestamptz
);
CREATE INDEX idx_admin_sessions_expires ON admin_sessions (expires_at);
