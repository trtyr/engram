-- 0045: project_files —— 项目文件（项目绑定的非 markdown 制品：架构图 HTML / 配置样例 / 导出报告等）
-- 渲染契约：mime=text/html 的文件前端走 iframe sandbox 查看器；text/markdown 走 WikiMarkdown；其余 <pre> 源码
-- 版本快照：同 name 覆盖更新时 version+1，旧内容进 project_file_versions（回滚依据，照 wiki_page_versions 先例）

CREATE TABLE IF NOT EXISTS project_files (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       text NOT NULL,
    mime       text NOT NULL DEFAULT 'text/plain',
    content    text NOT NULL DEFAULT '',
    version    integer NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT uniq_project_files_project_name UNIQUE (project_id, name),
    CONSTRAINT project_files_name_check CHECK (name <> '' AND length(name) <= 200 AND name NOT LIKE '%/%' AND name NOT LIKE '%\%')
);

CREATE INDEX IF NOT EXISTS idx_project_files_project ON project_files(project_id);

CREATE TABLE IF NOT EXISTS project_file_versions (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    file_id    uuid NOT NULL REFERENCES project_files(id) ON DELETE CASCADE,
    version    integer NOT NULL,
    content    text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT uniq_project_file_versions UNIQUE (file_id, version)
);
