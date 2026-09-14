-- 0043: 工单模型可用性——可读短号 + 关联关系
--
-- 1) short_no：全局单调短号（显示为 EN-<n>），sequence 生成（并发安全）
-- 2) todo_links：关联关系（blocked_by / relates_to / parent），
--    双 FK ON DELETE CASCADE（删 todo 自动清理关联行），from≠to 防自环

-- ---------- short_no ----------
ALTER TABLE todos ADD COLUMN short_no integer;

-- 回填：按 created_at（同刻按 id 稳定排序）编号既有行
WITH ranked AS (
    SELECT id, ROW_NUMBER() OVER (ORDER BY created_at, id) AS rn
    FROM todos
)
UPDATE todos SET short_no = ranked.rn FROM ranked WHERE todos.id = ranked.id;

CREATE SEQUENCE seq_todos_short_no OWNED BY todos.short_no;
-- 空表（clean DB）setval(1,false)=「1 尚未用」；有行 setval(max,true)=「max 已用」
SELECT setval('seq_todos_short_no',
              GREATEST((SELECT COALESCE(MAX(short_no), 0) FROM todos), 1),
              EXISTS(SELECT 1 FROM todos));
ALTER TABLE todos ALTER COLUMN short_no SET DEFAULT nextval('seq_todos_short_no');

CREATE UNIQUE INDEX idx_todos_short_no ON todos (short_no);

-- ---------- todo_links ----------
CREATE TABLE todo_links (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    from_id    uuid NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    to_id      uuid NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    kind       text NOT NULL CHECK (kind IN ('blocked_by', 'relates_to', 'parent')),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (from_id <> to_id),
    UNIQUE (from_id, to_id, kind)
);
CREATE INDEX idx_todo_links_from ON todo_links (from_id);
CREATE INDEX idx_todo_links_to ON todo_links (to_id);
