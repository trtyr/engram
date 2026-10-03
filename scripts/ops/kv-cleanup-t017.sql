-- T017 KV 存量大扫除（2026-10-03 摸底 9 条 → 2 搬家 1 核实删 6 合规留）
-- 部署配套脚本：生产执行前先跑末尾复核 SELECT；本脚本幂等（搬过的不重复搬）。
-- 注意：此为数据迁移非 schema 迁移，不入 migrations（sqlx 单向纪律）。

-- ① workrule_no_push_during_review → atoms（偏好/约定，纵向事实流）
--    该规则是「我的工作习惯」不是精确值——KV 判据（逐字必需）不满足。
INSERT INTO atoms (id, kind, content, confidence, status, needs_review, source_refs, strength, source_kind, tsv)
SELECT gen_random_uuid(),
       'convention',
       '工作规则：代码评审（needs_review 待审）期间不推代码，等评审结束再 push',
       0.9, 'active', false, '[]'::jsonb, 'fact', 'user_stated',
       to_tsvector('simple', '工作规则：代码评审 needs_review 待审 期间 不推 代码 等评审结束 再 push')
WHERE EXISTS (SELECT 1 FROM kv_entries WHERE key = 'workrule_no_push_during_review')
  AND NOT EXISTS (
    SELECT 1 FROM atoms WHERE status = 'active' AND content LIKE '%评审%期间不推代码%'
  );

-- ② agent-compose-beijing-hub → assets 域：hub 地址属基础设施台账。
--    assets 域有结构化 fields，脚本不代做（需人工/MCP 按台账模板登记）：
--    engram_assets put name=agent-compose-beijing-hub kind=service fields={url, role=matrix-hub}
--    登记完成后执行：
DELETE FROM kv_entries WHERE key = 'agent-compose-beijing-hub'
  AND EXISTS (SELECT 1 FROM assets WHERE name = 'agent-compose-beijing-hub');

-- ③ fleet-npm-registry-status → 核实后删（一次性状态值，会过期无归档价值）。
--    「核实」= 部署人确认 npm registry 状态已恢复正常或该探针已不再维护：
DELETE FROM kv_entries WHERE key = 'fleet-npm-registry-status';
--    如需保守：改用 UPDATE kv_entries SET ... 加 tombstone，或直接注释本行人工处理。

-- ④ 复核（执行后跑——预期：剩余 6 条 contact-*/resume-path/music-* 全部合规留）
SELECT key, updated_at FROM kv_entries ORDER BY updated_at DESC;
