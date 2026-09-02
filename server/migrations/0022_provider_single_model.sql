-- 供应商粒度重构：一个供应商一个模型一个 key。
-- chat 与 embedding 分开配置——models 数组拆成独立的单模型供应商行。

-- 1. 加新列（先给默认值，迁移数据后收紧）
ALTER TABLE llm_providers
  ADD COLUMN model_id text NOT NULL DEFAULT '',
  ADD COLUMN capability text NOT NULL DEFAULT 'chat';

-- 2. 单模型 provider：直接把数组唯一元素填充进新列
UPDATE llm_providers SET
    model_id = models->0->>'id',
    capability = COALESCE(models->0->'capabilities'->>0, 'chat')
WHERE jsonb_array_length(models) = 1;

-- 3. 多模型 provider：拆成多行（rn=1 保留原名，其余 name-capability）。
--    先物化到临时表 → 删原行（让出原名）→ 从临时表 INSERT，避开 name UNIQUE 冲突。
CREATE TEMP TABLE _split_providers AS
SELECT
    p.name, p.base_url, p.api_key_encrypted, p.is_default, p.created_at, p.updated_at,
    m.e->>'id' AS model_id,
    c.cap AS capability,
    row_number() OVER (PARTITION BY p.id ORDER BY (c.cap = 'chat') DESC, m.e->>'id', c.cap) AS rn
FROM llm_providers p
CROSS JOIN LATERAL jsonb_array_elements(p.models) AS m(e)
CROSS JOIN LATERAL jsonb_array_elements_text(m.e->'capabilities') AS c(cap)
WHERE jsonb_array_length(p.models) > 1;

DELETE FROM llm_providers WHERE jsonb_array_length(models) > 1;

INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default, created_at, updated_at)
SELECT
  gen_random_uuid(),
  CASE WHEN rn = 1 THEN name ELSE name || '-' || capability END,
  base_url, api_key_encrypted, model_id, capability, is_default, created_at, updated_at
FROM _split_providers;

DROP TABLE _split_providers;

-- 4. 删 models 列 + 收紧 capability
ALTER TABLE llm_providers DROP COLUMN models;
ALTER TABLE llm_providers ADD CONSTRAINT llm_providers_capability_check CHECK (capability IN ('chat', 'embedding'));
