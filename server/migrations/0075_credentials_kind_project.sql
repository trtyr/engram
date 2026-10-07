-- 0075: credentials 分类 + 项目绑定（凭据治理升级 2026-10-07）。
--
-- 背景：凭据此前只有 tags 平铺分组，用户拍板升级为两类正交维度：
--   · kind：封闭分类（写入必选）——账密/API/令牌/SSH/数据库/证书/服务器/支付/证件/笔记/自定义；
--   · project_id：可选项目绑定——个人/零散凭据不绑；项目删除时解绑留凭据（ON DELETE SET NULL，
--     与工单的强制绑定+级联删相反：凭据是资产不是工单，项目没了凭据还在）。
-- 存量凭据全部落 custom（无法推断历史意图，不猜）。

ALTER TABLE credentials
  ADD COLUMN kind text NOT NULL DEFAULT 'custom',
  ADD COLUMN project_id uuid REFERENCES projects(id) ON DELETE SET NULL;

ALTER TABLE credentials ADD CONSTRAINT credentials_kind_check CHECK (
  kind IN (
    'password',  -- 账号密码
    'api_key',   -- API Key / Secret
    'token',     -- 访问令牌（JWT / OAuth / PAT）
    'ssh_key',   -- SSH 密钥
    'database',  -- 数据库连接串
    'cert',      -- 证书 / 域名
    'server',    -- 服务器 / 主机凭据
    'payment',   -- 支付 / 银行卡
    'identity',  -- 证件 / 身份
    'note',      -- 安全笔记 / 恢复码
    'custom'     -- 自定义兜底
  )
);

CREATE INDEX IF NOT EXISTS idx_credentials_kind ON credentials (kind);
CREATE INDEX IF NOT EXISTS idx_credentials_project ON credentials (project_id);
