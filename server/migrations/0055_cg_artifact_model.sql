-- 0055: codegraph 产物模型（codegraph 上云线 task-2）。
--
-- 背景：README《核心模型》——一条 cg_projects = 一个项目 + 一份「当前有效产物」；
-- source_kind 只是「当前这份产物从哪来」的标注，**不是项目身份**。两个入口
-- （云端自建 cloud_index / 客户端上传 client_upload）都只是给这条记录投递新产物的管道，
-- 因此 0051 那条「repo 型拒绝 upload 覆盖」的规则随之上移到应用层撤销（该规则只在
-- 「服务端自己管着那个 path」的本地实例成立，云端不成立）。
--
-- ① source_kind 值域改成语义化命名（与文档口径统一）：
--      repo   → cloud_index   （云端拉仓库跑 index/sync）
--      upload → client_upload （客户端本机 index 后上传产物）
--    并同步改列默认值（register 不显式写该列，靠默认值）。
-- ② 产物元数据三件套，用于「这份产物是谁、什么时候、用什么版本产出的」可追溯：
--      produced_at        产物产出时间（云自建 = 索引完成时刻；上传 = 客户端声明/入库时刻）
--      built_with_version 产出该产物的 CLI 版本（上传侧从 db 内 project_metadata 校验后写入）
--      last_producer      本次投递者：cloud_index 或 client:<key 名>
ALTER TABLE cg_projects DROP CONSTRAINT IF EXISTS cg_projects_source_kind_check;

UPDATE cg_projects
SET source_kind = CASE source_kind
    WHEN 'repo' THEN 'cloud_index'
    WHEN 'upload' THEN 'client_upload'
    ELSE source_kind
END;

ALTER TABLE cg_projects ALTER COLUMN source_kind SET DEFAULT 'cloud_index';

ALTER TABLE cg_projects ADD CONSTRAINT cg_projects_source_kind_check
    CHECK (source_kind IN ('cloud_index', 'client_upload'));

ALTER TABLE cg_projects ADD COLUMN IF NOT EXISTS produced_at timestamptz;
ALTER TABLE cg_projects ADD COLUMN IF NOT EXISTS built_with_version text;
ALTER TABLE cg_projects ADD COLUMN IF NOT EXISTS last_producer text;
