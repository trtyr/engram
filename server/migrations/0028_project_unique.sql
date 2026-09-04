-- 0028：项目记忆域唯一约束——防同名项目 + 同项目同分类同名文档
CREATE UNIQUE INDEX IF NOT EXISTS uniq_projects_name ON projects(name);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_project_docs_title ON project_docs(project_id, category, title);
