-- 0036: wiki 页面版本历史（R 报告建议 #5：wiki 只有版本计数器无历史——
-- 覆盖即失忆，删页连最后状态都找不回）。
-- 写入口（put_page 覆盖分支、delete_page）先把现状快照进本表；恢复 = 从快照落回。
-- 保留策略：每 slug 最近 50 版（与 skill_revisions 同口径），插入侧裁剪。

CREATE TABLE wiki_page_versions (
    id         uuid PRIMARY KEY,
    slug       text NOT NULL,
    version    int  NOT NULL,
    title      text NOT NULL,
    page_type  text NOT NULL,
    folder     text NOT NULL DEFAULT '',
    content    text NOT NULL,
    origin     text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_wiki_page_versions_slug ON wiki_page_versions (slug, version DESC);
CREATE INDEX idx_wiki_page_versions_time ON wiki_page_versions (created_at DESC);
