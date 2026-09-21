-- 代码图谱入口收敛（2026-09-21）：注册落盘方式标记。
--
-- dest_mode ∈ {default, custom}：
--   default = 服务端自建目录（`<数据根>/codegraph/<项目名>`，重名加 `-2`）→ 删条目时**连目录清**；
--   custom  = 用户指定父目录（`<父目录>/<仓库名>`）→ 删条目时**只删注册与产物、目录保留**。
--
-- 为什么要落库而不是「按路径猜」：删除是否连目录清是**破坏性**决定，必须由「注册时怎么定的」
-- 决定；靠路径前缀猜会在自定义目录恰好在数据根之内时误删用户数据。
--
-- 历史行一律回填 custom（= 保护）：既有条目里既有本地路径型（用户自己的源码目录），
-- 也有早先 clone 到 uuid 目录的——升级不能因为我们的新语义去删用户的目录。
ALTER TABLE cg_projects
    ADD COLUMN IF NOT EXISTS dest_mode text NOT NULL DEFAULT 'custom';

-- 约束：值域二值（幂等：pg_constraint 存在则跳过）
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'cg_projects_dest_mode_check'
    ) THEN
        ALTER TABLE cg_projects
            ADD CONSTRAINT cg_projects_dest_mode_check
            CHECK (dest_mode IN ('default', 'custom'));
    END IF;
END $$;

-- 新注册由代码显式传值；表默认值改回 default 只作兜底（贴合新语义：服务端自建为主路径）。
ALTER TABLE cg_projects ALTER COLUMN dest_mode SET DEFAULT 'default';
