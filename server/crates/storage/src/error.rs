//! 仓储层错误。SQL 失败统一收口为 [`StoreError`]；
//! 「不存在」语义由仓储函数返回 `Option`、服务层判定，不在此编码。

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("存储暂时不可用: {0}")]
    Sql(#[from] sqlx::Error),
    /// 唯一约束冲突（23505）。服务层映射为各自的 Conflict/BadRequest 语义。
    #[error("唯一约束冲突: {0}")]
    Conflict(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

/// sqlx UNIQUE 冲突判定（PG 23505）。
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}
