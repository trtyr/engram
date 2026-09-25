-- 0059 实体同名不敏感唯一（EN-242 审计补强，2026-09-25）：
-- 旧 uniq_entities_name_kind 是 (name, kind) 精确匹配——'helm'/'Helm' 大小写异形各自成档
-- （EN-242 实测症状）。改表达式唯一索引：归一化名（lower+btrim）+ kind 在活体（未合并未归档）内唯一。
-- 同时纳入 archived_at——合并归档的副档不占名，允许未来重建同名实体。
DROP INDEX IF EXISTS uniq_entities_name_kind;
CREATE UNIQUE INDEX uniq_entities_name_kind
    ON entities(lower(btrim(name)), kind)
    WHERE merged_into IS NULL AND archived_at IS NULL;
