-- 0007: Wiki 域 — 原料 / 页面 / 链接图（见 topics/wiki-engine.md）。
CREATE TABLE wiki_sources (
    id               uuid PRIMARY KEY,
    sha256           text NOT NULL UNIQUE,
    raw_path         text NOT NULL,
    title            text,
    status           text NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending','processing','ready','failed')),
    last_ingested_at timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE wiki_pages (
    id          uuid PRIMARY KEY,
    slug        text NOT NULL UNIQUE,
    title       text NOT NULL,
    page_type   text NOT NULL CHECK (page_type IN
                ('entity','concept','source','synthesis','comparison','overview','index','log')),
    content     text NOT NULL,               -- Markdown
    frontmatter jsonb NOT NULL DEFAULT '{}', -- 含 sources[]
    origin      text NOT NULL DEFAULT 'llm' CHECK (origin IN ('llm','human')),
    version     int NOT NULL DEFAULT 1,
    embedding   vector(1024),
    tsv         tsvector,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_wiki_pages_type ON wiki_pages (page_type);
CREATE INDEX idx_wiki_pages_tsv ON wiki_pages USING gin (tsv);
CREATE INDEX idx_wiki_pages_embedding ON wiki_pages USING hnsw (embedding vector_cosine_ops);

-- 图边：wikilink 边（weight 3.0）与 source-overlap 边（weight 4.0）
CREATE TABLE wiki_links (
    from_slug text NOT NULL,
    to_slug   text NOT NULL,
    weight    real NOT NULL DEFAULT 3.0,
    PRIMARY KEY (from_slug, to_slug)
);
CREATE INDEX idx_wiki_links_to ON wiki_links (to_slug);
