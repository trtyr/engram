-- 0008: CodeGraph 域 — 项目注册表（图谱数据由 codegraph 自管于 data/codegraph/）。
CREATE TABLE cg_projects (
    id             uuid PRIMARY KEY,
    name           text NOT NULL UNIQUE,
    path           text NOT NULL,       -- data/codegraph/<id>/ 工作目录
    source_uri     text NOT NULL,       -- 注册时的本地路径或 git URL
    status         text NOT NULL DEFAULT 'registered'
                   CHECK (status IN ('registered','indexing','ready','error','version_mismatch')),
    stats          jsonb,               -- {files, symbols, edges}
    error          text,
    last_synced_at timestamptz,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);
