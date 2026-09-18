-- 0051: codegraph 产物上传模型（公网多Agent P001 步骤4）。
-- upload 型条目：服务端只存 db 产物 + 声明式新鲜度（head/uploaded_at），
-- 无代码、无 git 凭证——客户端本机 codegraph CLI index 后上传产物。
ALTER TABLE cg_projects ADD COLUMN source_kind text NOT NULL DEFAULT 'repo'
    CHECK (source_kind IN ('repo', 'upload'));
ALTER TABLE cg_projects ADD COLUMN head text;
ALTER TABLE cg_projects ADD COLUMN uploaded_at timestamptz;
