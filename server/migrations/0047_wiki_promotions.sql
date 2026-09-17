-- 0047: wiki_promotions —— 项目文档 → wiki 的知识晋升登记（EN-59）
-- 晋升 = 复制非搬迁：项目文档保留项目语境版本，wiki 是提炼后的通用版本（synthesis 页）。
-- 本表承载双向可查的结构化关系；wiki 页 frontmatter（promoted_from）与项目文档标记行
-- 是给人/查询看的镜像，非唯一事实源。
-- UNIQUE(project_id, doc_id, page_slug)：同一来源文档的同一页只登记一次——重复晋升
-- 同一 (project, doc, page) 视为已存在（服务层转友好报错）。
CREATE TABLE wiki_promotions (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    library_id  uuid NOT NULL REFERENCES wiki_libraries(id) ON DELETE CASCADE,
    page_slug   text NOT NULL,
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    doc_id      uuid NOT NULL REFERENCES project_docs(id) ON DELETE CASCADE,
    anchor      text NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, doc_id, page_slug)
);

CREATE INDEX idx_wiki_promotions_page ON wiki_promotions (library_id, page_slug);
CREATE INDEX idx_wiki_promotions_project ON wiki_promotions (project_id);
