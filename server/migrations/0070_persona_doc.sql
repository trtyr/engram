-- P015 离线整理：画像活文档（单行 + 历史版本链）
-- 整理 Agent 定期维护一份 Markdown 画像文档；每次编辑旧版进 history（可审计可回滚）。
CREATE TABLE persona_doc (
    id INT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    content TEXT NOT NULL,
    version INT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE persona_doc_history (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    version INT NOT NULL,
    content TEXT NOT NULL,
    summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
