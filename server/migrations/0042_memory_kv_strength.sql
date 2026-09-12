-- 0042: 记忆可靠性——KV 值保值通道 + atoms 断言强度
--
-- 1) kv_entries：结构化 KV 存储（工单「值不是一等公民」）。
--    - value 逐字保存，蒸馏管道永不触碰——机器产出（序列号/UUID/IP 等）原样透传
--    - key 唯一 UPSERT 就地更新（工单「可变状态不适合 append-only 取代链」）
--    - updated_at 天然承载「最后校验时间」（工单「状态变更无人发现」的第一块基石）
-- 2) atoms 加 strength/source_kind 断言强度两列（工单「断言强度缺失」）。
--    - strength: fact=用户明示/机器验证, inference=agent 推断, assumption=假设
--    - source_kind: 值从哪来（user_stated/verified_probe/agent_inferred/doc）
--    - DDL 默认 fact 兜底存量语义；蒸馏代码路径显式传 inference（新产出保守）

-- ---------- kv_entries ----------
CREATE TABLE IF NOT EXISTS kv_entries (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    key         text NOT NULL UNIQUE,
    value       text NOT NULL,
    context     text NOT NULL DEFAULT '',
    tags        text[] NOT NULL DEFAULT '{}',
    source      text NOT NULL DEFAULT 'user_stated'
                CHECK (source IN ('user_stated','verified_probe','agent_inferred','doc')),
    tsv         tsvector GENERATED ALWAYS AS (
                  to_tsvector('simple', coalesce(key, '') || ' ' || value)
                ) STORED,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_kv_tsv ON kv_entries USING gin (tsv);

-- ---------- atoms 断言强度 ----------
ALTER TABLE atoms ADD COLUMN IF NOT EXISTS strength text NOT NULL DEFAULT 'fact'
    CHECK (strength IN ('fact','inference','assumption'));
ALTER TABLE atoms ADD COLUMN IF NOT EXISTS source_kind text NOT NULL DEFAULT 'agent_inferred'
    CHECK (source_kind IN ('user_stated','verified_probe','agent_inferred','doc'));
