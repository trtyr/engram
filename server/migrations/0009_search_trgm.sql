-- 0009: 检索补位 — pg_trgm（子串/模糊匹配，D0009 的 C 部分）。
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- 中文长文本的 trgm 索引成本可控（知识域分块后体量小）；
-- atoms/scenarios/wiki_pages 的 tsv 已有 GIN，trgm 主要服务 chunks 与短语查询。
CREATE INDEX idx_chunks_content_trgm ON chunks USING gin (content gin_trgm_ops);
CREATE INDEX idx_atoms_content_trgm ON atoms USING gin (content gin_trgm_ops);
