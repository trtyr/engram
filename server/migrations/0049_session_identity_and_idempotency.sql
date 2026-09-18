-- 0049: 会话身份归因 + 幂等键（公网多Agent P001 步骤1，2026-09-17）
-- api_key_id：归因到哪把 key（哪类 Agent 角色）；key 删除不级联删会话（SET NULL）。
-- key_name_snapshot：key 名冗余快照——key 改名/删除后历史归因仍可读。
-- client_ref：客户端幂等键（同一 ref 重写返回原会话，防网络重试产生重复记忆）。
ALTER TABLE raw_sessions ADD COLUMN api_key_id uuid REFERENCES api_keys(id) ON DELETE SET NULL;
ALTER TABLE raw_sessions ADD COLUMN key_name_snapshot text;
ALTER TABLE raw_sessions ADD COLUMN client_ref text;
CREATE UNIQUE INDEX idx_raw_sessions_client_ref ON raw_sessions (client_ref) WHERE client_ref IS NOT NULL;
