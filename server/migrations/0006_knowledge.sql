-- 0006: 知识域 — 文档与分块（见 topics/knowledge-ingest.md）。
CREATE TABLE documents (
    id         uuid PRIMARY KEY,
    title      text NOT NULL,
    source_uri text NOT NULL,           -- 上传文件名或 URL
    mime       text,
    raw_path   text,                    -- data/uploads/ 下的路径
    sha256     text UNIQUE,
    status     text NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending','parsing','chunking','embedding','ready','failed')),
    error      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_documents_status ON documents (status);
CREATE INDEX idx_documents_created ON documents (created_at DESC);

CREATE TABLE chunks (
    id           uuid PRIMARY KEY,
    document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    seq          int NOT NULL,
    content      text NOT NULL,
    embed_failed boolean NOT NULL DEFAULT false,
    embedding    vector(1024),
    tsv          tsvector,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (document_id, seq)
);
CREATE INDEX idx_chunks_tsv ON chunks USING gin (tsv);
CREATE INDEX idx_chunks_embedding ON chunks USING hnsw (embedding vector_cosine_ops);
