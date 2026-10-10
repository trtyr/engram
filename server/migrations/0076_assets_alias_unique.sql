-- 0076: assets 别名唯一性 DB 兑底（P019-M4）。
-- 旧实现别名唯一性只有应用层先查后写（find_by_name_or_alias 预检），注释声称
-- 「并发窗口由 UNIQUE 兜底」但 UNIQUE 只覆盖 name——并发 create/update 可让两条
-- 资产同持一个别名，此后按别名解析结果不确定。用触发器做行级校验（text[] 数组
-- 无法用普通 UNIQUE 索引表达元素级唯一）：大小写不敏感，包内重复一并拒绝。
CREATE OR REPLACE FUNCTION assets_aliases_conflict() RETURNS trigger AS $$
BEGIN
    -- 包内重复
    IF (SELECT count(*) FROM unnest(NEW.aliases) x)
       <> (SELECT count(DISTINCT lower(x)) FROM unnest(NEW.aliases) x) THEN
        RAISE EXCEPTION 'asset aliases 包内重复（大小写不敏感）: %', NEW.aliases;
    END IF;
    -- 与其它资产冲突
    IF EXISTS (
        SELECT 1
        FROM assets a, unnest(a.aliases) AS ea
        WHERE a.id <> NEW.id
          AND ea IN (SELECT lower(x) FROM unnest(NEW.aliases) x)
    ) OR EXISTS (
        SELECT 1 FROM assets a
        WHERE a.id <> NEW.id AND lower(a.name) IN (SELECT lower(x) FROM unnest(NEW.aliases) x)
    ) THEN
        RAISE EXCEPTION 'asset alias 与既有资产冲突: %', NEW.aliases;
    END IF;
    RETURN NEW;
END $$ LANGUAGE plpgsql;

CREATE TRIGGER trg_assets_aliases_conflict
BEFORE INSERT OR UPDATE OF aliases, name ON assets
FOR EACH ROW EXECUTE FUNCTION assets_aliases_conflict();
