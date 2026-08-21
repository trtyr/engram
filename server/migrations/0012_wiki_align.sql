-- 0012: Wiki 对齐 llm_wiki — Review 系统 + 洞察 dismiss + purpose 配置。
CREATE TABLE wiki_review_items (
    id             uuid PRIMARY KEY,
    kind           text NOT NULL CHECK (kind IN ('create_page','deep_research','skip','flag')),
    payload        jsonb NOT NULL DEFAULT '{}',   -- 标题/理由/建议内容等
    action         text,                          -- 处理时选择的动作（label）
    search_queries jsonb NOT NULL DEFAULT '[]',   -- LLM 预生成的检索词
    source_id      uuid REFERENCES wiki_sources(id) ON DELETE SET NULL,
    status         text NOT NULL DEFAULT 'open' CHECK (status IN ('open','resolved','dismissed')),
    created_at     timestamptz NOT NULL DEFAULT now(),
    resolved_at    timestamptz
);
CREATE INDEX idx_wiki_review_open ON wiki_review_items (created_at) WHERE status = 'open';

CREATE TABLE wiki_insight_dismissals (
    insight_key text PRIMARY KEY,       -- 稳定键（类型:slug 对）
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- purpose 存 settings（key/value 复用 0010），无新表。
