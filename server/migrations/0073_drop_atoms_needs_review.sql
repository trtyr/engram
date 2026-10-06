-- 0073: atoms.needs_review 列退役——待审通道随 P015 蓝图 v2 全链退役
-- （写入只追加，低置信产出由整理 Agent 巡逻处置，不再有「等人判定」的待审态）
ALTER TABLE atoms DROP COLUMN IF EXISTS needs_review;
