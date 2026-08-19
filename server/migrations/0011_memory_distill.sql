-- 0011: 记忆域蒸馏支持 — atoms 增加 candidate 状态（extract 产物，arbitrate 前的暂存）
-- 与 scenario_id 归属列（organize 阶段回填；NULL = 未归组）。
ALTER TABLE atoms DROP CONSTRAINT atoms_status_check;
ALTER TABLE atoms ADD CONSTRAINT atoms_status_check CHECK (status IN
    ('candidate','active','superseded','archived'));

ALTER TABLE atoms ADD COLUMN scenario_id uuid REFERENCES scenarios(id) ON DELETE SET NULL;
CREATE INDEX idx_atoms_ungrouped ON atoms (created_at)
    WHERE status = 'active' AND scenario_id IS NULL;
CREATE INDEX idx_atoms_candidate ON atoms (created_at)
    WHERE status = 'candidate';
