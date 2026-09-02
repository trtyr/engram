-- 处置直写残留：无 source_refs 的 active 原子打标 origin=direct-write（溯源断但可审计）
-- 权限收窄后 AI 只写会话，直写只剩用户 Web 手动补充；这些是历史直写/重建残留
-- sentinel 不含 session_id，erase_session 的 LIKE 匹配与 purge_agent 的 jsonb 遍历均不会误伤
UPDATE atoms
SET source_refs = '[{"origin": "direct-write"}]'::jsonb
WHERE status = 'active'
  AND (source_refs IS NULL OR source_refs = '[]'::jsonb);
