-- 0031: 技能域（第六域）— AI 技能（SKILL.md 形态）的资产化管理。
-- skills（本体：slug 唯一 + frontmatter 字段拉平 + markdown 正文）
-- / skill_revisions（版本快照：每次语义变更前留快照，可回滚，保留最近 50 版）。

CREATE TABLE skills (
    id          uuid PRIMARY KEY,
    slug        text NOT NULL UNIQUE,              -- URL/MCP 标识，kebab-case
    name        text NOT NULL,                     -- 显示名（frontmatter name）
    description text NOT NULL DEFAULT '',          -- 一句话说明（frontmatter description，注入 prompt 时的选择依据）
    content     text NOT NULL DEFAULT '',          -- markdown 正文（技能指令本体）
    tags        text[] NOT NULL DEFAULT '{}',      -- 自由标签
    enabled     boolean NOT NULL DEFAULT true,     -- 停用 = 对 AI 隐身（列表/检索不出现）
    -- 来源：manual=Web/API 手建 / import=批量导入 / mcp=MCP 工具面写入
    source      text NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','import','mcp')),
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_skills_enabled ON skills (enabled);
CREATE INDEX idx_skills_tags ON skills USING gin (tags);
CREATE INDEX idx_skills_updated ON skills (updated_at DESC);

-- 版本快照：语义字段（name/description/content/tags）变更前整快照；restore 也先快照现状再落历史版。
CREATE TABLE skill_revisions (
    id         uuid PRIMARY KEY,
    skill_id   uuid NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    rev        integer NOT NULL,                   -- 技能内递增版本号（从 1 起）
    name       text NOT NULL,
    description text NOT NULL,
    content    text NOT NULL,
    tags       text[] NOT NULL DEFAULT '{}',
    origin     text NOT NULL CHECK (origin IN ('create','update','restore')),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (skill_id, rev)
);
CREATE INDEX idx_skill_revisions_skill ON skill_revisions (skill_id, rev DESC);
