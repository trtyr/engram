-- 0065: study 学习路线图跟踪器（P007-T001）——学习过程的路线图状态机。
-- 设计：领域 track → 知识单元 item（状态机 待学/进行中/已学）→ 挂 wiki 页/documents。
-- 分工：study 只存「学没学/学到哪/下一步学啥」的过程状态；知识内容归 wiki（harness 维护），
-- 原文归 documents，感悟叙事归 memory（蒸馏判据同步见 P007-T005）。
-- 单库终局：无 library_id（wiki 单库 main，引用全指向它）。
CREATE TABLE study_tracks (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    goal        text NOT NULL DEFAULT '',  -- 学到什么程度算完（归档锚）
    status      text NOT NULL DEFAULT 'active'
                CHECK (status IN ('active','paused','done')),
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE study_track_items (
    id          uuid PRIMARY KEY,
    track_id    uuid NOT NULL REFERENCES study_tracks(id) ON DELETE CASCADE,
    name        text NOT NULL,
    status      text NOT NULL DEFAULT 'not_started'
                CHECK (status IN ('not_started','learning','learned')),
    position    int NOT NULL DEFAULT 0,    -- 路线图顺序（next_up 派生排序）
    wiki_slugs  jsonb NOT NULL DEFAULT '[]', -- 关联 wiki 页 ["slug", ...]
    doc_ids     jsonb NOT NULL DEFAULT '[]', -- 关联文档 [doc_id, ...]
    learned_at  timestamptz,               -- 置 learned 时写入
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_study_items_track ON study_track_items (track_id, position);
