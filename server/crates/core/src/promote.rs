//! 项目文档 → wiki 知识晋升（EN-59）：跨 wiki + projects 两域的结构化动作。
//!
//! 晋升 = **复制非搬迁**：源文档保留项目语境版本（服务端自动追加晋升标记——
//! frontmatter.promoted 数组 + 正文末尾 ⛳ 标记行），wiki 侧落调用方 AI 提炼后的
//! 通用版本（synthesis 页，frontmatter 带 promoted_from 源回链）。登记表
//! wiki_promotions 承载双向可查的结构化关系。
//! 判定口径见《文档工作流》晋升节（三问：离开本项目还成立吗 / 别的项目用得上吗 /
//! 是对世界的陈述还是项目历史）——判定由调用方 AI 在写作时执行，服务端不做 LLM 提炼。

use engram_storage::PgPool;
use engram_storage::repo::{project as project_repo, wiki_promotions as promo_repo};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PromoteError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(#[from] engram_storage::StoreError),
    #[error("{0}")]
    Wiki(#[from] engram_wiki_engine::WikiError),
}

/// 晋升请求。`content` 是调用方 AI **提炼后的通用知识正文**（服务端不做 LLM 提炼）。
#[derive(Debug, serde::Deserialize)]
pub struct PromoteRequest {
    /// 来源项目（名或 id）
    pub project: String,
    /// 来源文档 id
    pub doc_id: Uuid,
    /// 源定位（小节标题/行区间说明——回链精度用，如 "§机器产出原样透传"）
    #[serde(default)]
    pub anchor: String,
    /// 目标页 slug
    pub slug: String,
    /// 页标题（提炼后的通用标题，非原文标题）
    pub title: String,
    /// 提炼后的通用知识正文（markdown，可带 [[wikilink]]）
    pub content: String,
    /// 目标库（缺省 main）
    pub library: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct PromoteOutcome {
    pub library: String,
    pub page_slug: String,
    pub page_title: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub doc_id: Uuid,
}

/// 晋升编排（写 synthesis 页 + 登记行 + 源文档标记双写）。
pub struct PromoteService {
    pool: PgPool,
}

impl PromoteService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn promote(&self, req: PromoteRequest) -> Result<PromoteOutcome, PromoteError> {
        if req.title.trim().is_empty() || req.content.trim().is_empty() {
            return Err(PromoteError::BadRequest(
                "title 与 content（提炼正文）都不能为空——提炼由调用方 AI 完成，服务端不做 LLM 提炼"
                    .into(),
            ));
        }

        // ① 源解析：项目（名或 id）+ 文档（必须属于该项目）
        let project_id = match Uuid::parse_str(&req.project) {
            Ok(id) => id,
            Err(_) => project_repo::id_by_name(&self.pool, &req.project)
                .await?
                .ok_or_else(|| {
                    PromoteError::NotFound(format!(
                        "项目「{}」不存在——先 projects 列表确认名字",
                        req.project
                    ))
                })?,
        };
        let project_name = self.resolve_promote_source(&req, project_id).await?;

        let (lib, lib_slug) = self.register_promotion(&req, project_id).await?;

        // ④ 写 synthesis 页（wiki-engine 域内：版本快照 + wikilinks 重算同口径）
        let promoted_from = format!("project:{}/{}#{}", project_name, req.doc_id, req.anchor);
        let page = engram_wiki_engine::promote::promote_page(
            &self.pool,
            lib,
            &req.slug,
            &req.title,
            &req.content,
            &promoted_from,
        )
        .await;

        // ⑤ 源文档标记双写（登记已成功；页面写入失败则回滚登记，不留半态）
        let page = match page {
            Ok(p) => p,
            Err(e) => {
                let _ = promo_repo::delete_by_page(&self.pool, lib, &req.slug).await; // 有意忽略：登记回滚 best-effort，主错误已向上抛
                return Err(e.into());
            }
        };
        project_repo::mark_doc_promoted(
            &self.pool,
            req.doc_id,
            &format!("{}/{}", lib_slug, req.slug),
            &req.anchor,
        )
        .await?;

        Ok(PromoteOutcome {
            library: lib_slug,
            page_slug: page.slug,
            page_title: page.title,
            project_id,
            project_name,
            doc_id: req.doc_id,
        })
    }

    /// 晋升登记列表：按项目（名或 id）过滤，或全量（缺省）。
    pub async fn list_promotions(
        &self,
        project: Option<&str>,
    ) -> Result<Vec<engram_storage::models::wiki_promotions::WikiPromotionDto>, PromoteError> {
        let project_id = match project {
            Some(p) => match Uuid::parse_str(p) {
                Ok(id) => id,
                Err(_) => project_repo::id_by_name(&self.pool, p)
                    .await?
                    .ok_or_else(|| {
                        PromoteError::NotFound(format!(
                            "项目「{p}」不存在——先 projects 列表确认名字"
                        ))
                    })?,
            },
            None => return promo_repo::list_all(&self.pool).await.map_err(Into::into),
        };
        promo_repo::list_by_project(&self.pool, project_id)
            .await
            .map_err(Into::into)
    }

    /// ② 目标库解析（缺省 main）+ ③ 登记先行（幂等闸门：重复晋升在写页前友好报「已晋升」）。
    async fn register_promotion(
        &self,
        req: &PromoteRequest,
        project_id: Uuid,
    ) -> Result<(Uuid, String), PromoteError> {
        // ② 目标库解析（缺省 main）
        let lib = engram_wiki_engine::libraries::resolve(&self.pool, req.library.as_deref())
            .await
            .map_err(|e| PromoteError::NotFound(e.to_string()))?;
        let lib_slug = promo_repo::library_slug(&self.pool, lib).await?;

        // ③ 登记先行（幂等闸门：重复晋升在写页之前就友好报「已晋升」）
        promo_repo::insert(
            &self.pool,
            lib,
            &req.slug,
            project_id,
            req.doc_id,
            &req.anchor,
        )
        .await
        .map_err(|e| match e {
            engram_storage::StoreError::Conflict(_) => PromoteError::Conflict(format!(
                "已晋升过：文档 {} 的「{}」已登记为 wiki:{}/{}——同一来源同一页只登记一次；\
                     若要更新提炼内容直接改目标页（write_page）",
                req.doc_id, req.anchor, lib_slug, req.slug
            )),
            other => PromoteError::from(other),
        })?;
        Ok((lib, lib_slug))
    }
    /// ① 源解析收尾：文档必须属于该项目；项目名优先，兜底文档标题（回链展示用）。
    async fn resolve_promote_source(
        &self,
        req: &PromoteRequest,
        project_id: Uuid,
    ) -> Result<String, PromoteError> {
        let doc = project_repo::get_doc(&self.pool, req.doc_id)
            .await?
            .ok_or_else(|| {
                PromoteError::NotFound(format!(
                    "文档 {} 不存在——先 project-get <项目> 看文档列表取 id",
                    req.doc_id
                ))
            })?;
        if doc.project_id != project_id {
            return Err(PromoteError::BadRequest(format!(
                "文档 {} 不属于项目 {}——请核对来源",
                req.doc_id, req.project
            )));
        }
        let project_name = doc.title.clone(); // 回链展示用项目名优先，兜底文档标题
        let project_name =
            match engram_storage::repo::project::get_project(&self.pool, project_id).await {
                Ok(Some(p)) => p.name,
                _ => project_name,
            };
        Ok(project_name)
    }
}
