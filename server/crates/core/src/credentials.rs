//! 凭据域服务：机密值的一等台账（按名存取 + 静态加密 + 取用审计，EN-234）。
//!
//! **设计**（2026-09-24 用户拍板「凭据独立成域」）：
//! 凭据的安全等级高于一切精确值——此前混在 memory KV 通道里无审计无生命周期，现独立成域：
//!   · 值用 KeyCipher（AGENT_MEMORY_MASTER_KEY 体系，与 LLM provider 密钥同源）静态加密落库；
//!   · 按名取用：`get(name)` 解密返回直接可用值，同时留审计（credential_reads + 计数器）；
//!   · 列表/元数据永不回显值——值只在 get 响应里出现一次；
//!   · 值更换即清零旧取用审计（值换了，旧痕作废）。
//!
//! 纪律：凭据明文只允许出现在 get 响应里，禁止落入任何日志/文档/工单正文。

use engram_llm::KeyCipher;
use engram_storage::models::credential::{
    CredentialMetaDto, CredentialReadRow, CredentialValueDto,
};
use engram_storage::repo::credential as repo;
use engram_storage::{PgPool, StoreError};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("主密钥不可用，无法加解密凭据：{0}")]
    Crypto(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for CredentialError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(_) => {
                CredentialError::Conflict("唯一约束冲突（同名凭据已存在）".into())
            }
            StoreError::Sql(e) => CredentialError::Storage(e.to_string()),
        }
    }
}

pub struct CredentialsService {
    pool: PgPool,
    cipher: KeyCipher,
}

impl CredentialsService {
    pub fn new(pool: PgPool, cipher: KeyCipher) -> Self {
        Self { pool, cipher }
    }

    fn enc(&self, plaintext: &str) -> Result<Vec<u8>, CredentialError> {
        self.cipher
            .encrypt(plaintext)
            .map_err(|e| CredentialError::Crypto(e.to_string()))
    }

    fn dec(&self, data: &[u8]) -> Result<String, CredentialError> {
        self.cipher
            .decrypt(data)
            .map_err(|e| CredentialError::Crypto(e.to_string()))
    }

    fn validate_name(name: &str) -> Result<String, CredentialError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CredentialError::BadRequest(
                "凭据名不能为空——按名取用靠它定位（如 newapi/api_key）".into(),
            ));
        }
        if name.len() > 200 {
            return Err(CredentialError::BadRequest("凭据名过长（>200）".into()));
        }
        Ok(name.to_string())
    }

    /// 写入/更新（同名换值即清零旧取用审计）。
    pub async fn put(
        &self,
        name: &str,
        value: &str,
        description: Option<&str>,
        created_by: &str,
    ) -> Result<CredentialMetaDto, CredentialError> {
        let name = Self::validate_name(name)?;
        if value.is_empty() {
            return Err(CredentialError::BadRequest("凭据值不能为空".into()));
        }
        let enc = self.enc(value)?;
        Ok(repo::upsert(
            &self.pool,
            Uuid::now_v7(),
            &name,
            &enc,
            true,
            description.unwrap_or(""),
            created_by,
        )
        .await?)
    }

    /// 按名取用：解密返回直接可用值，并留取用审计。
    pub async fn get(
        &self,
        name: &str,
        reader: &str,
    ) -> Result<CredentialValueDto, CredentialError> {
        let name = Self::validate_name(name)?;
        let row = repo::get_enc_by_name(&self.pool, &name)
            .await?
            .ok_or_else(|| {
                CredentialError::NotFound(format!(
                    "没有名为「{name}」的凭据——先 credentials list 看台账，或用 put 建档"
                ))
            })?;
        let value = self.dec(&row.value_enc)?;
        repo::record_read(&self.pool, row.id, reader).await?;
        Ok(CredentialValueDto {
            name: row.name,
            value,
            sensitive: row.sensitive,
            description: row.description,
            last_read_at: Some(chrono::Utc::now()),
            read_count: row.read_count + 1,
        })
    }

    /// 台账（元数据，永不回显值）。
    pub async fn list(&self) -> Result<Vec<CredentialMetaDto>, CredentialError> {
        Ok(repo::list(&self.pool).await?)
    }

    /// 取用审计流水（最近在前）。
    pub async fn reads(&self, name: &str) -> Result<Vec<CredentialReadRow>, CredentialError> {
        let name = Self::validate_name(name)?;
        Ok(repo::list_reads(&self.pool, &name).await?)
    }

    /// 删除（级联清取用审计）。
    pub async fn delete(&self, name: &str) -> Result<bool, CredentialError> {
        let name = Self::validate_name(name)?;
        Ok(repo::delete(&self.pool, &name).await? > 0)
    }
}
