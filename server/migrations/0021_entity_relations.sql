-- 0021 实体关系（2026-09-01 圈子强化 P3）：类型化关系表，图从「共现抱团」升级成有向知识图谱。
-- 方向语义：from --rel_type--> to（如 张三 --member_of--> 后端组）。
-- 同向同类型唯一（weight 累加）；反向是不同关系，不冲突。
CREATE TABLE IF NOT EXISTS entity_relations (
    id uuid PRIMARY KEY,
    from_id uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    to_id uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    rel_type text NOT NULL CHECK (rel_type IN ('member_of', 'located_in', 'works_on', 'part_of', 'related_to')),
    weight integer NOT NULL DEFAULT 1,
    source text NOT NULL DEFAULT 'manual' CHECK (source IN ('distill', 'manual')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_entity_relations_from ON entity_relations(from_id);
CREATE INDEX IF NOT EXISTS idx_entity_relations_to ON entity_relations(to_id);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_entity_relation ON entity_relations(from_id, to_id, rel_type);
