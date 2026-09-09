-- 0039: project_docs 支持子文件夹（folder）——分类内的多级目录。
-- folder 为相对路径（/ 分隔），'' 表示直接挂在分类根下；
-- 树形呈现 = category → folder 路径节点 → 文档。前端左树递归渲染。

ALTER TABLE project_docs ADD COLUMN folder text NOT NULL DEFAULT '';

-- 同分类同路径下标题唯一（原约束只看 category+title，路径化后放宽到不同 folder 可同名）。
-- 0028 建的是 UNIQUE INDEX（无 pg_constraint 行），且手工迁移史可能产生具名 CONSTRAINT——
-- 两种形态都按列组合动态定位再删，避免任一环境下静默残留。
DO $$
DECLARE r record;
BEGIN
    -- ① unique index 形态
    FOR r IN
        SELECT i.relname AS idxname
        FROM pg_class t
        JOIN pg_index ix ON ix.indrelid = t.oid AND ix.indisunique AND NOT ix.indisprimary
        JOIN pg_class i ON i.oid = ix.indexrelid
        WHERE t.relname = 'project_docs'
          AND pg_get_indexdef(ix.indexrelid) LIKE '%(project_id, category, title)%'
    LOOP
        EXECUTE format('DROP INDEX %I', r.idxname);
    END LOOP;
    -- ② unique constraint 形态
    FOR r IN
        SELECT conname FROM pg_constraint
        WHERE conrelid = 'project_docs'::regclass AND contype = 'u'
          AND pg_get_constraintdef(oid) LIKE '%UNIQUE (project_id, category, title)%'
    LOOP
        EXECUTE format('ALTER TABLE project_docs DROP CONSTRAINT %I', r.conname);
    END LOOP;
END $$;
ALTER TABLE project_docs ADD CONSTRAINT uniq_project_docs_folder_title
    UNIQUE (project_id, category, folder, title);
CREATE INDEX idx_project_docs_folder ON project_docs (project_id, category, folder);

-- 存量回填：按 title 前缀映射初始 folder（决定论映射，无歧义）
UPDATE project_docs SET folder = 'Wiki' WHERE title LIKE 'Wiki · %';
UPDATE project_docs SET folder = '审计' WHERE title LIKE '审计 · %';
UPDATE project_docs SET folder = '后端增强' WHERE title LIKE 'backend-enhancement · %';
UPDATE project_docs SET folder = '归档/ai-permissions' WHERE title LIKE 'ai-permissions · %';
UPDATE project_docs SET folder = '归档/circle' WHERE title LIKE 'circle · %';
UPDATE project_docs SET folder = '归档/frontend-polish' WHERE title LIKE 'frontend-polish · %';
UPDATE project_docs SET folder = '归档/memory-rhythm' WHERE title LIKE 'memory-rhythm · %';
UPDATE project_docs SET folder = '归档/project-memory' WHERE title LIKE 'project-memory · %';
UPDATE project_docs SET folder = '归档/wiki-unify' WHERE title LIKE 'wiki-unify · %';
UPDATE project_docs SET folder = '归档/backend-e2e' WHERE title = 'backend-e2e · 开放问题';
UPDATE project_docs SET folder = '归档/wiki-theory-integration' WHERE title LIKE 'wiki-theory-integration · %';
UPDATE project_docs SET folder = '归档' WHERE title IN (
    '后端 E2E · roadmap', 'wiki 理论整合 · roadmap', 'skills 二态改造 · roadmap',
    '后端增强 · roadmap（归档）');
UPDATE project_docs SET folder = '记录' WHERE title LIKE '文档迁移记录%' OR title LIKE '文档整理记录%';
