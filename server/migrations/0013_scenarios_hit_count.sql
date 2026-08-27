-- 0013: scenarios 增加 hit_count（B9 检索命中回写）。
-- 与 atoms.hit_count 同语义：被检索命中的使用热度；consolidate stale 降权依据。
ALTER TABLE scenarios ADD COLUMN hit_count int NOT NULL DEFAULT 0;
