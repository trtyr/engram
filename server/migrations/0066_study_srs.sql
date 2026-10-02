-- P007 二期 T011/T012：SRS 复习字段 + journal 进度时间线表
ALTER TABLE study_track_items
    ADD COLUMN needs_review BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN review_due_at TIMESTAMPTZ;

CREATE TABLE study_track_journal (
    id UUID PRIMARY KEY,
    track_id UUID NOT NULL REFERENCES study_tracks(id) ON DELETE CASCADE,
    note TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_study_journal_track ON study_track_journal (track_id, created_at DESC);
