-- 0020 实体摘要版本链（2026-09-01 圈子强化）：手编实体档案的轻量历史。
-- 复用原子 revisions 模式（0018）：错字修正/档案改写留旧值痕，非双条记账。
CREATE TABLE IF NOT EXISTS entity_revisions (
    id uuid PRIMARY KEY,
    entity_id uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    old_summary text NOT NULL,
    edited_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_entity_revisions_entity ON entity_revisions(entity_id, created_at DESC);
