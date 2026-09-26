-- 0062: 资产域运维手册——runbook 列 + 修订史表（资产强化：主机运维台账，2026-09-26）。
--
-- 语义：每资产一份 Markdown 运行手册（主动记录，非实时采集）——硬件规格/磁盘/网络/跑的服务/
-- 端口/部署位置/依赖/变更记录/踩坑。结构化字段（ip/os/fields）答「它是什么」，
-- runbook_md 答「它现在什么情况、怎么运维」——看一眼资产即知全貌。
-- 改写留修订史（复用 0020 entity_revisions 模式）：错改可回滚，变更可追溯。

ALTER TABLE assets ADD COLUMN IF NOT EXISTS runbook_md text NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS asset_revisions (
    id uuid PRIMARY KEY,
    asset_id uuid NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    old_runbook_md text NOT NULL,
    edited_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_asset_revisions_asset ON asset_revisions(asset_id, created_at DESC);
