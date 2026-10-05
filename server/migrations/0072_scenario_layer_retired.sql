-- P015 场景层退役（蓝图 v2 收官刀）：
-- ① scenarios 表 drop——L2 场景层整体退役（organize 链已下架，离线整理 Agent 接管其职责）
-- ② atoms.scenario_id 列 drop——原子不再归属场景（写入只追加哲学）
-- ③ persona_aspects 表 drop——旧分面画像退役，画像唯一形态 = persona_doc 活文档
-- 不可逆：生产执行前以 pre-agentic 备份兜底。
DROP TABLE IF EXISTS scenarios CASCADE;
ALTER TABLE atoms DROP COLUMN IF EXISTS scenario_id;
DROP TABLE IF EXISTS persona_aspects CASCADE;
