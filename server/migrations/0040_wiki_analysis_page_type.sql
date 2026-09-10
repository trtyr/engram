-- 0040：wiki_pages.page_type 增加-analysis（karpathy LLM Wiki 的 query 回填：
-- 问答/分析产物归档为正式页面，与 synthesis「跨源综合」语义不同——
-- analysis 是「对既有页面集合的分析产物」）。仅放宽 CHECK，不改任何数据行。
ALTER TABLE wiki_pages DROP CONSTRAINT wiki_pages_page_type_check;
ALTER TABLE wiki_pages ADD CONSTRAINT wiki_pages_page_type_check
  CHECK ((page_type = ANY (ARRAY['entity'::text, 'concept'::text, 'source'::text, 'synthesis'::text, 'comparison'::text, 'queries'::text, 'overview'::text, 'index'::text, 'log'::text, 'purpose'::text, 'analysis'::text])));
