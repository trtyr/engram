-- 实体：记忆的主角（人物/项目/主题/群组）——以用户为中心的记忆星系节点。
-- 蒸馏抽取时自动产出；同名同类只保留一个活体（merge 后置 merged_into，让出唯一槽位）。
CREATE TABLE entities (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    kind        text NOT NULL CHECK (kind IN ('person', 'project', 'topic', 'group')),
    summary     text NOT NULL DEFAULT '',
    merged_into uuid REFERENCES entities(id),
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_entities_kind ON entities(kind);
-- 活体唯一（merge 掉的不占名——同名实体可以重新出现并接管）
CREATE UNIQUE INDEX uniq_entities_name_kind ON entities(name, kind) WHERE merged_into IS NULL;

-- 原子 ↔ 实体关联（多对多；图谱的边由共现关系推导，不单建关系表）
CREATE TABLE atom_entities (
    atom_id    uuid NOT NULL REFERENCES atoms(id) ON DELETE CASCADE,
    entity_id  uuid NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (atom_id, entity_id)
);
CREATE INDEX idx_atom_entities_entity ON atom_entities(entity_id);
