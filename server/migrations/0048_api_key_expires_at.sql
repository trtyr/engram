-- EN-62 ③：API key 过期机制——NULL = 永不过期（存量 key 完全兼容）。
ALTER TABLE api_keys ADD COLUMN expires_at timestamptz;

-- EN-62 ④：scopes 列默认移除——签发/更新走 API 层显式必填，消灭「裸 INSERT 悄悄拿到三域权限」的隐式授权
-- （表默认曾是 ["memory","wiki","codegraph"]，与 API 层/前端默认各不一致——三处三个样是困惑源头）。
ALTER TABLE api_keys ALTER COLUMN scopes DROP DEFAULT;
