-- 0038: 技能二态改造——按附属文件是否含「真脚本」分 text / script 两种存储形态。
-- text：无脚本，或依赖走 npm/cargo 全局二进制（文件夹内全是文本）——整体入库，现状不变。
-- script：真身只存本地文件夹（SKILL.md + scripts/ 等），系统只存指针（local_path）+ 来源；
--         content 不入库（get 时现读）、file_*/versions/restore 拒绝、不产 revisions 快照。
-- 既有行默认 kind='text' / origin='self'，历史数据零迁移成本。

-- 存储形态：text=入库 / script=本地指针
ALTER TABLE skills ADD COLUMN kind text NOT NULL DEFAULT 'text'
    CHECK (kind IN ('text','script'));
-- 来源：self=自建未发布 / github=源自 GitHub / both=自建且已发布（纯元数据，不做远端拉取）
ALTER TABLE skills ADD COLUMN origin text NOT NULL DEFAULT 'self'
    CHECK (origin IN ('self','github','both'));
ALTER TABLE skills ADD COLUMN local_path text;  -- script 型指针：本地技能文件夹绝对路径
ALTER TABLE skills ADD COLUMN repo_url text;    -- origin 含 github 时的仓库地址

-- 指针与入库正文互斥：script 必须有 local_path；text 不得有
ALTER TABLE skills ADD CONSTRAINT chk_skills_kind_path
    CHECK (
        (kind = 'script' AND local_path IS NOT NULL AND local_path <> '')
        OR (kind = 'text' AND local_path IS NULL)
    );
-- 注：script 型的 skill_files 必须为空——跨表约束放应用层（core）校验。
