-- 0017: P3 sensitive 隐私标记 + P5 void 会话作废（测试方第三波 · 用户 2026-08-31 批准全项）
-- sensitive：医疗/感情/财务类原子默认不进检索与 context_pack；显式 reveal 才可见。
-- 隐私判定交给写入者（AI 直写/人工），蒸馏不自动标——LLM 判隐私不可信。
-- void：「这段白记了」——蒸馏跳过（claim 只取 pending）、记录保留。

ALTER TABLE atoms ADD COLUMN sensitive boolean NOT NULL DEFAULT false;
CREATE INDEX idx_atoms_sensitive ON atoms (sensitive) WHERE sensitive;

ALTER TABLE raw_sessions DROP CONSTRAINT raw_sessions_distill_status_check;
ALTER TABLE raw_sessions ADD CONSTRAINT raw_sessions_distill_status_check
    CHECK (distill_status IN ('pending', 'processing', 'done', 'void'));
