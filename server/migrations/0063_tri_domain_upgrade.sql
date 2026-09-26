-- 0063: 凭据/待办/工单三域补齐（2026-09-27，资产强化同款打法）。
--
-- 一、credentials：tags（按系统/环境分组）+ expires_at（过期治理——key 会过期，
--     临期/已过期在台账页高亮，过期判定在查询端按 now() 算）。
-- 二、工单活动时间线：工单（todos 表 kind='ticket'）的状态流转与评论统一入
--     ticket_events——kind='event' 由核心在流转时自动写（from→to+actor），
--     kind='comment' 承载讨论。工单删除级联清时间线。

ALTER TABLE credentials
    ADD COLUMN IF NOT EXISTS tags text[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS expires_at timestamptz;
CREATE INDEX IF NOT EXISTS idx_credentials_tags ON credentials USING gin (tags);

CREATE TABLE IF NOT EXISTS ticket_events (
    id        uuid PRIMARY KEY,
    ticket_id uuid NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    kind      text NOT NULL CHECK (kind IN ('event', 'comment')),
    payload   jsonb NOT NULL DEFAULT '{}',  -- event: {from,to,note} / comment: {text}
    actor     text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ticket_events_ticket ON ticket_events(ticket_id, created_at DESC);
