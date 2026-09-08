-- 0037: Wiki 多库（library）——用户定版 A 方案：真正的多库隔离。
-- 库 = wiki 的一级命名空间：页面/双链/原料/文档/审查/洞察/purpose 全部挂库。
-- 存量数据回填进默认库 main（主库）。

CREATE TABLE wiki_libraries (
    id         uuid PRIMARY KEY,
    slug       text NOT NULL UNIQUE,
    name       text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO wiki_libraries (id, slug, name)
SELECT gen_random_uuid(), 'main', '主库'
WHERE NOT EXISTS (SELECT 1 FROM wiki_libraries WHERE slug = 'main');

-- 通用回填套路：加列 → 回填 main → NOT NULL → FK → 索引/约束换装

-- 1) wiki_pages：slug 唯一从全局降为库内唯一（跨库同名页面合法）
ALTER TABLE wiki_pages ADD COLUMN library_id uuid;
UPDATE wiki_pages SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_pages ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_pages ADD CONSTRAINT fk_wiki_pages_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
ALTER TABLE wiki_pages DROP CONSTRAINT wiki_pages_slug_key;
ALTER TABLE wiki_pages ADD CONSTRAINT uniq_wiki_pages_lib_slug UNIQUE (library_id, slug);
CREATE INDEX idx_wiki_pages_library ON wiki_pages (library_id);

-- 2) wiki_sources：sha 去重降为库内（同原料可在不同库各织一份）
ALTER TABLE wiki_sources ADD COLUMN library_id uuid;
UPDATE wiki_sources SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_sources ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_sources ADD CONSTRAINT fk_wiki_sources_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
ALTER TABLE wiki_sources DROP CONSTRAINT wiki_sources_sha256_key;
ALTER TABLE wiki_sources ADD CONSTRAINT uniq_wiki_sources_lib_sha UNIQUE (library_id, sha256);
CREATE INDEX idx_wiki_sources_library ON wiki_sources (library_id);

-- 3) wiki_links：PK 纳入库维度（slug 跨库可重名）
ALTER TABLE wiki_links ADD COLUMN library_id uuid;
UPDATE wiki_links SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_links ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_links ADD CONSTRAINT fk_wiki_links_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
ALTER TABLE wiki_links DROP CONSTRAINT wiki_links_pkey;
ALTER TABLE wiki_links ADD CONSTRAINT wiki_links_pkey PRIMARY KEY (library_id, from_slug, to_slug);

-- 4) wiki_page_versions：快照按库隔离（slug 跨库可重名）
ALTER TABLE wiki_page_versions ADD COLUMN library_id uuid;
UPDATE wiki_page_versions SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_page_versions ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_page_versions ADD CONSTRAINT fk_wiki_page_versions_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
DROP INDEX IF EXISTS idx_wiki_page_versions_slug;
CREATE INDEX idx_wiki_page_versions_lib ON wiki_page_versions (library_id, slug, version DESC);

-- 5) wiki_review_items：人审队列按库分列
ALTER TABLE wiki_review_items ADD COLUMN library_id uuid;
UPDATE wiki_review_items SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_review_items ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_review_items ADD CONSTRAINT fk_wiki_review_items_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
CREATE INDEX idx_wiki_review_items_library ON wiki_review_items (library_id);

-- 6) wiki_insight_dismissals：洞察键跨库同名——PK 纳入库维度
ALTER TABLE wiki_insight_dismissals ADD COLUMN library_id uuid;
UPDATE wiki_insight_dismissals SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_insight_dismissals ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_insight_dismissals ADD CONSTRAINT fk_wiki_insight_dismissals_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
ALTER TABLE wiki_insight_dismissals DROP CONSTRAINT wiki_insight_dismissals_pkey;
ALTER TABLE wiki_insight_dismissals ADD CONSTRAINT wiki_insight_dismissals_pkey
    PRIMARY KEY (library_id, insight_key);

-- 7) wiki_documents / wiki_chunks：文档知识子系统同库隔离
ALTER TABLE wiki_documents ADD COLUMN library_id uuid;
UPDATE wiki_documents SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_documents ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_documents ADD CONSTRAINT fk_wiki_documents_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
ALTER TABLE wiki_documents DROP CONSTRAINT wiki_documents_sha256_key;
ALTER TABLE wiki_documents ADD CONSTRAINT uniq_wiki_documents_lib_sha UNIQUE (library_id, sha256);
CREATE INDEX idx_wiki_documents_library ON wiki_documents (library_id);

ALTER TABLE wiki_chunks ADD COLUMN library_id uuid;
UPDATE wiki_chunks SET library_id = (SELECT id FROM wiki_libraries WHERE slug = 'main');
ALTER TABLE wiki_chunks ALTER COLUMN library_id SET NOT NULL;
ALTER TABLE wiki_chunks ADD CONSTRAINT fk_wiki_chunks_library
    FOREIGN KEY (library_id) REFERENCES wiki_libraries(id) ON DELETE CASCADE;
CREATE INDEX idx_wiki_chunks_library ON wiki_chunks (library_id);

-- 8) purpose 迁到每库一份（settings key = wiki_purpose:{lib_id}）：
--    旧全局 key 的值回填给主库
INSERT INTO settings (key, value)
SELECT 'wiki_purpose:' || l.id, s.value
FROM wiki_libraries l
JOIN settings s ON s.key = 'wiki_purpose'
WHERE l.slug = 'main'
  AND NOT EXISTS (SELECT 1 FROM settings WHERE key = 'wiki_purpose:' || l.id);
