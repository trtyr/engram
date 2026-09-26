-- 0060: credentials 凭据域（EN-234）。
--
-- 背景（2026-09-24 用户拍板「凭据独立成域」，见工单 EN-234；2026-09-26 goal muht4x5v 落地）：
-- 凭据（API Key / Token）此前混在 memory 域的 KV 通道里，与普通精确值同域存放，无集中管理、
-- 无独立生命周期、无取用审计。凭据的安全等级高于一切精确值，故独立成域：
--   · 值静态加密（复用 LLM provider 的 KeyCipher / AGENT_MEMORY_MASTER_KEY 体系）；
--   · 按名取用（get <name> 返回直接可用值），取用即留痕（读审计表 + 计数器）；
--   · sensitive 内建（列表/元数据永不回显值，值只在 get 响应中出现一次）。
--
-- 纪律：凭据明文只允许出现在 get 的响应里，禁止落入任何日志 / 文档 / 工单正文。

CREATE TABLE IF NOT EXISTS credentials (
    id            uuid PRIMARY KEY,
    name          text NOT NULL,                 -- 按名取用的唯一键（btrim，大小写不敏感命中走查询端 lower）
    value_enc     bytea NOT NULL,                -- KeyCipher(AES-GCM) 加密后的值；明文永不落库
    sensitive     boolean NOT NULL DEFAULT true, -- 敏感标记（内建，列表/元数据回显标记不回显值）
    description   text NOT NULL DEFAULT '',      -- 用途说明（可含「用在哪/找谁」等元信息，不含值）
    created_by    text NOT NULL DEFAULT '',      -- 建档者（审计元信息）
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    last_read_at  timestamptz,                   -- 取用审计：最近一次 get
    read_count    integer NOT NULL DEFAULT 0     -- 取用审计：累计 get 次数
);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_credentials_name ON credentials (lower(btrim(name)));

-- 取用审计流水：每次 get 一行（谁、何时、取了哪条）。凭据删除时级联清审计（凭据不存在了，痕随主档）。
CREATE TABLE IF NOT EXISTS credential_reads (
    id            uuid PRIMARY KEY,
    credential_id uuid NOT NULL REFERENCES credentials(id) ON DELETE CASCADE,
    reader        text NOT NULL DEFAULT '',
    read_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_credential_reads_cred ON credential_reads (credential_id, read_at DESC);
