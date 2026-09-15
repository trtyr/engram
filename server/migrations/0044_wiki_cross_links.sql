-- 0044：wiki 跨库引用（R：wiki 多库补全——[[lib/slug]] 语法）
-- 库内引用继续走 wiki_links（library_id + from/to_slug，语义不变）；
-- 跨库引用单独成表：from 页（库 A slug X）引用 to 页（库 B slug Y）。
-- 双向可见（to 页 backlinks 联查）+ 级联清理（删页/删库）。
CREATE TABLE IF NOT EXISTS wiki_cross_links (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    from_library_id uuid NOT NULL REFERENCES wiki_libraries(id) ON DELETE CASCADE,
    from_slug       text NOT NULL,
    to_library_id   uuid NOT NULL REFERENCES wiki_libraries(id) ON DELETE CASCADE,
    to_slug         text NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (from_library_id, from_slug, to_library_id, to_slug)
);
CREATE INDEX IF NOT EXISTS idx_wiki_cross_links_to
    ON wiki_cross_links (to_library_id, to_slug);
