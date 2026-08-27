-- 0014: Wiki — wiki_sources 补 error 列（W4：analyze/generate Permanent 失败可落原因）。
ALTER TABLE wiki_sources ADD COLUMN error text;
