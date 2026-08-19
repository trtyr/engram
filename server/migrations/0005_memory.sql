-- 0005: 记忆域 — L0 会话 / L1 原子 / L2 场景 / L3 画像（见 topics/memory-model.md）。
-- embedding 统一 1024 维（bge-m3，D0010）；tsv 由应用层预分词维护（D0009）。

-- L0 原始会话（不可变）
CREATE TABLE raw_sessions (
    id            uuid PRIMARY KEY,
    agent         text NOT NULL DEFAULT 'default',
    content       jsonb NOT NULL,            -- [{speaker, text, ts}]
    metadata      jsonb NOT NULL DEFAULT '{}',
    distill_status text NOT NULL DEFAULT 'pending'
                  CHECK (distill_status IN ('pending','processing','done')),
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_raw_sessions_created ON raw_sessions (created_at DESC);
CREATE INDEX idx_raw_sessions_distill ON raw_sessions (distill_status) WHERE distill_status = 'pending';

-- L1 原子记忆
CREATE TABLE atoms (
    id            uuid PRIMARY KEY,
    kind          text NOT NULL CHECK (kind IN
                  ('preference','fact','decision','event','insight','correction','failure','convention')),
    content       text NOT NULL,
    confidence    real NOT NULL DEFAULT 0.8,
    source_refs   jsonb NOT NULL DEFAULT '[]',   -- [{session_id, span}]
    status        text NOT NULL DEFAULT 'active'
                  CHECK (status IN ('active','superseded','archived')),
    superseded_by uuid REFERENCES atoms(id),
    needs_review  boolean NOT NULL DEFAULT false,
    hit_count     int NOT NULL DEFAULT 0,
    embedding     vector(1024),
    tsv           tsvector,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_atoms_active_kind ON atoms (kind) WHERE status = 'active';
CREATE INDEX idx_atoms_status ON atoms (status);
CREATE INDEX idx_atoms_tsv ON atoms USING gin (tsv);
CREATE INDEX idx_atoms_embedding ON atoms USING hnsw (embedding vector_cosine_ops);

-- L2 场景知识块
CREATE TABLE scenarios (
    id          uuid PRIMARY KEY,
    topic       text NOT NULL,
    summary     text NOT NULL,
    body        text NOT NULL,
    atom_refs   jsonb NOT NULL DEFAULT '[]',
    version     int NOT NULL DEFAULT 1,
    embedding   vector(1024),
    tsv         tsvector,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_scenarios_tsv ON scenarios USING gin (tsv);
CREATE INDEX idx_scenarios_embedding ON scenarios USING hnsw (embedding vector_cosine_ops);

-- L3 画像分面（版本化，可回滚）
CREATE TABLE persona_aspects (
    id            uuid PRIMARY KEY,
    aspect        text NOT NULL CHECK (aspect IN
                  ('identity','preferences','skills','constraints','communication_style','goals','routines')),
    content       text NOT NULL,
    evidence_refs jsonb NOT NULL DEFAULT '[]',
    version       int NOT NULL DEFAULT 1,
    prompt_version text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);
-- 每个 aspect 的全部历史版本（当前版本 = 每 aspect 最大 version）
CREATE UNIQUE INDEX idx_persona_aspect_version ON persona_aspects (aspect, version);
