-- 批次② 查询日志飞轮（wiki 大库化 2026-09-19）：
-- 每次检索 UPSERT 一行——「你的搜索日志知道知识库缺什么」（Karpathy 社区洞见）：
-- 零命中 = 内容缺口；低分（仅单通道末位水平）= 召回质量存疑。
-- 缺口清单（GET /wiki/query-gaps）供织入方向与 Deep Research 消费。
CREATE TABLE IF NOT EXISTS wiki_query_log (
    id uuid PRIMARY KEY,
    library_id uuid NOT NULL REFERENCES wiki_libraries(id) ON DELETE CASCADE,
    query text NOT NULL,
    calls integer NOT NULL DEFAULT 0,
    zero_calls integer NOT NULL DEFAULT 0,
    low_calls integer NOT NULL DEFAULT 0,
    last_top_score real,
    last_queried_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (library_id, query)
);
CREATE INDEX IF NOT EXISTS idx_wiki_query_log_gaps
    ON wiki_query_log (library_id, zero_calls, low_calls);
