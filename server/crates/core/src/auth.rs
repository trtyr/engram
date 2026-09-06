//! 鉴权身份与 scope 模型（跨 HTTP/MCP 共用； Formerly api::auth）。
//!
//! Bearer 中间件（axum）留在 api 层；本模块只放协议无关的身份与 scope 语义，
//! 供 api 路由与 mcp 工具面共用同一套分权检查。

use uuid::Uuid;

/// 资产域 scope。
pub const SCOPES: [&str; 9] = [
    "memory",
    "wiki",
    "codegraph",
    "project",
    "skills",
    "todos",
    "llm",
    "erase",
    "cron",
];

/// 已认证主体。
#[derive(Debug, Clone)]
pub enum Principal {
    /// 管理员（Web UI 会话，全权限）
    Admin,
    /// API key 机器主体（限 scopes）
    ApiKey {
        key_id: Uuid,
        name: String,
        scopes: Vec<String>,
    },
}

impl Principal {
    /// 是否拥有某 scope（Admin 恒真）。
    pub fn has_scope(&self, scope: &str) -> bool {
        match self {
            Principal::Admin => true,
            Principal::ApiKey { scopes, .. } => scopes.iter().any(|s| s == scope),
        }
    }
}

// ---------- 管理员账号（单用户；0033） ----------
//
// 完整的用户名 + 密码登录。密码 PBKDF2-HMAC-SHA256（120k 轮）哈希落库，
// 格式 `pbkdf2-sha256$iter$salt$hash`。兼容链：admin_account 表为空时——
// env 密码已设则自动播种（username='admin'）；env 未设则由登录页初始化表单
// 创建账号。表非空后以表为准（env 失效）。

use engram_storage::PgPool;
use engram_storage::repo::keys as keys_repo;
use serde_json::Value;

const PBKDF2_ROUNDS: u32 = 120_000;

/// PBKDF2-HMAC-SHA256 哈希，格式 `pbkdf2-sha256$iter$salt$hash`（hex）。
pub fn hash_password(password: &str) -> String {
    use sha2::Sha256;
    let salt: [u8; 16] = rand_random_salt();
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ROUNDS, &mut out);
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!("pbkdf2-sha256${PBKDF2_ROUNDS}${}${}", hex(&salt), hex(&out))
}

fn rand_random_salt() -> [u8; 16] {
    // 复用 rand（api 层同款）；core 不直接依赖 rand 的话经 std 随机不可靠——
    // 直接使用 rand crate（workspace 已有）
    use rand::RngCore;
    let mut s = [0u8; 16];
    rand::rng().fill_bytes(&mut s);
    s
}

/// 校验密码 against 存储格式（兼容格式前缀解析与轮次）。
pub fn verify_password(password: &str, stored: &str) -> bool {
    use sha2::Sha256;
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2-sha256" {
        return false;
    }
    let Ok(rounds) = parts[1].parse::<u32>() else {
        return false;
    };
    let (Ok(salt), Ok(expected)) = (hex_decode(parts[2]), hex_decode(parts[3])) else {
        return false;
    };
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, rounds, &mut out);
    out[..] == expected[..]
}

fn hex_decode(s: &str) -> Result<Vec<u8>, ()> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| ()))
        .collect()
}

/// 账号是否已创建。
pub async fn account_exists(pool: &PgPool) -> Result<bool, String> {
    Ok(keys_repo::get_admin_account(pool)
        .await
        .map_err(|e| e.to_string())?
        .is_some())
}

/// env 密码播种：表空且 env 有值 → 创建 username='admin'。返回是否播种。
pub async fn seed_account_if_empty(
    pool: &PgPool,
    env_password: Option<&str>,
) -> Result<bool, String> {
    if account_exists(pool).await? {
        return Ok(false);
    }
    match env_password.map(str::trim).filter(|s| !s.is_empty()) {
        Some(pw) => {
            keys_repo::upsert_admin_account(pool, "admin", &hash_password(pw))
                .await
                .map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 登录校验：账号存在 → username + pbkdf2；账号不存在 → env 密码回退（兼容旧部署）。
pub async fn verify_login(
    pool: &PgPool,
    username: &str,
    password: &str,
    env_fallback: Option<&str>,
) -> Result<bool, String> {
    match keys_repo::get_admin_account(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        Some((user, hash)) => Ok(user == username && verify_password(password, &hash)),
        None => Ok(username == "admin"
            && env_fallback
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|env| constant_time_eq_str(env, password))
                .unwrap_or(false)),
    }
}

fn constant_time_eq_str(a: &str, b: &str) -> bool {
    use sha2::{Digest, Sha256};
    let da = Sha256::digest(a.as_bytes());
    let db = Sha256::digest(b.as_bytes());
    da.as_slice() == db.as_slice()
}

/// 初始化账号（仅表空时有效；否则返回 false）。
pub async fn init_account(pool: &PgPool, username: &str, password: &str) -> Result<bool, String> {
    if account_exists(pool).await? {
        return Ok(false);
    }
    keys_repo::upsert_admin_account(pool, username, &hash_password(password))
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 当前用户名（未初始化 → None）。
pub async fn get_username(pool: &PgPool) -> Result<Option<String>, String> {
    Ok(keys_repo::get_admin_account(pool)
        .await
        .map_err(|e| e.to_string())?
        .map(|(u, _)| u))
}

/// 修改凭证（改用户名/密码，至少一项）：校验当前密码 → 覆盖 → 吊销其他会话。
/// 返回 Err(描述) 于校验失败；(false, _) = 当前密码错误。
pub async fn change_credentials(
    pool: &PgPool,
    current_password: &str,
    new_username: Option<&str>,
    new_password: Option<&str>,
    current_token_hash: &str,
) -> std::result::Result<(bool, Value), String> {
    use serde_json::json;
    let Some((username, hash)) = keys_repo::get_admin_account(pool)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Err("账号尚未初始化".into());
    };
    if !verify_password(current_password, &hash) {
        return Ok((false, json!({ "error": "当前密码错误" })));
    }
    let new_username = new_username.map(str::trim).filter(|s| !s.is_empty());
    if let Some(u) = new_username {
        if u.len() < 3 || u.len() > 32 {
            return Err("用户名需 3~32 字符".into());
        }
        if !u
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Err("用户名仅允许字母/数字/_-.".into());
        }
    }
    if let Some(pw) = new_password
        && pw.chars().count() < 8
    {
        return Err("新密码至少 8 位".into());
    }
    let username = new_username.unwrap_or(&username);
    let hash = match new_password {
        Some(pw) => hash_password(pw),
        None => hash,
    };
    keys_repo::upsert_admin_account(pool, username, &hash)
        .await
        .map_err(|e| e.to_string())?;
    let revoked = keys_repo::delete_other_admin_sessions(pool, current_token_hash)
        .await
        .map_err(|e| e.to_string())?;
    Ok((
        true,
        json!({ "username": username, "revoked_sessions": revoked }),
    ))
}
