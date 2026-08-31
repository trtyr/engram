-- 0018 编辑能力（2026-08-31，用户拍板）：真人可改、AI 追加——账本与钉住机制。
-- 1) 原子轻量版本历史：改写语义（content/kind/confidence）编辑前的旧值留痕。
--    AI 走 correction（新原子+superseded_by）不进这张表；这是"错字修正不值得双条"的轻量路。
CREATE TABLE IF NOT EXISTS atom_revisions (
    id uuid PRIMARY KEY,
    atom_id uuid NOT NULL REFERENCES atoms(id) ON DELETE CASCADE,
    old_content text NOT NULL,
    old_kind text NOT NULL,
    old_confidence real NOT NULL,
    edited_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_atom_revisions_atom ON atom_revisions(atom_id, created_at DESC);

-- 2) 用户钉住：手编的分面/实体档案，蒸馏彻底绕开（确定性排除，不靠模型自觉）。
--    F4 清退优先级更高：removed_texts 命中的手编内容照样清。
ALTER TABLE persona_aspects ADD COLUMN IF NOT EXISTS manually_edited boolean NOT NULL DEFAULT false;
ALTER TABLE entities ADD COLUMN IF NOT EXISTS manually_edited boolean NOT NULL DEFAULT false;
