-- 0032: 技能文件夹形态（folder skill）——SKILL.md 之外的附属文件。
-- skill = 文件夹：skills.content 是 SKILL.md 本体；scripts/、references/、assets/ 等
-- 附属文件按相对路径寻址存本表。云部署语义：文件是「内容」不是「文件系统位置」——
-- MCP 按路径下发，AI 客户端取走后本地执行，服务端永不执行任何上传代码。

CREATE TABLE skill_files (
    id         uuid PRIMARY KEY,
    skill_id   uuid NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    path       text NOT NULL,                      -- 相对路径（/ 分隔，禁止 .. 与绝对路径）
    content    text NOT NULL,                      -- 文本内容（脚本/参考资料/模板）
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (skill_id, path)
);
CREATE INDEX idx_skill_files_skill ON skill_files (skill_id, path);
