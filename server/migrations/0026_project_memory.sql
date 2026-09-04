-- 0026: 项目记忆域 — 项目 / 位置 / 分类文档（见 docs/plantree/plans/project-memory/）。
-- 三表模型（0005）：projects（本体 + 分类列表）/ project_locations（多主机位置，登记制）
-- / project_docs（分类 > markdown 文档）。类型模板（dev·research 的预设分类）是代码常量，
-- 新建项目时复制进 projects.categories，之后项目级自由增删。

CREATE TABLE projects (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    type        text NOT NULL CHECK (type IN ('dev','research')),
    -- status 英文枚举，Web 层映射中文：active=进行中 / paused=暂停 / done=完成 / abandoned=放弃
    status      text NOT NULL DEFAULT 'active'
                CHECK (status IN ('active','paused','done','abandoned')),
    description text,
    categories  jsonb NOT NULL DEFAULT '[]',   -- 分类名列表（中文，如 ["后端","前端","测试","规划"]），可增删
    frontmatter jsonb NOT NULL DEFAULT '{}',   -- 轻结构扩展（字段集后续设计时定）
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_projects_type ON projects (type);
CREATE INDEX idx_projects_created ON projects (created_at DESC);

-- 位置数组：{主机, 路径, 用途} 多行，纯元数据登记（不远程读取主机）。
CREATE TABLE project_locations (
    id         uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    host       text NOT NULL,                  -- 主机名（MacBook Pro / tencent-beijing 等）
    path       text NOT NULL,                  -- 文件夹路径
    purpose    text,                           -- 用途（开发 / 部署 / ...）
    sort_order int NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_project_locations_project ON project_locations (project_id);

-- 分类文档：category 是项目级分类名（引用 projects.categories 里的值），正文纯 markdown。
CREATE TABLE project_docs (
    id          uuid PRIMARY KEY,
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    category    text NOT NULL,                 -- 分类名（后端 / 前端 / 测试 / 规划 / 待查 / ...）
    title       text NOT NULL,
    content     text NOT NULL DEFAULT '',      -- Markdown 正文
    frontmatter jsonb NOT NULL DEFAULT '{}',   -- 轻结构元数据（状态标签等）
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_project_docs_project ON project_docs (project_id, category);
