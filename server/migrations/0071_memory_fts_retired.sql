-- P015 阶段三：记忆域 FTS 退役——检索纯向量一路。
-- atoms/scenarios 的 tsv 列与 GIN 索引退役；wiki 域（chunks/pages）不受影响。
DROP INDEX IF EXISTS idx_atoms_tsv;
ALTER TABLE atoms DROP COLUMN IF EXISTS tsv;
DROP INDEX IF EXISTS idx_scenarios_tsv;
ALTER TABLE scenarios DROP COLUMN IF EXISTS tsv;
