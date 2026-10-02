//! Study 学习路线图服务（P007-T003）：学习过程的路线图状态机。
//!
//! 分工边界：study 只管「学没学/学到哪/下一步学啥」的过程状态；
//! 知识内容归 wiki（harness 维护），原文归 documents，感悟叙事归 memory。

use engram_storage::PgPool;
use uuid::Uuid;

use engram_storage::repo;

/// topic_get 全量返回（跨会话恢复学习上下文的核心契约：
/// 一次拿全【进度+下一步队列+资料清单】）。
#[derive(Debug, serde::Serialize)]
pub struct TopicFull {
    #[serde(flatten)]
    pub track: repo::study::StudyTrackRow,
    /// 全部节点（position ASC）
    pub items: Vec<repo::study::StudyItemRow>,
    /// 进度（总数/已学）
    pub progress: Progress,
    /// 下一步队列（not_started 按 position 前 5）
    pub next_up: Vec<repo::study::StudyItemRow>,
    /// 进行中节点
    pub in_progress: Vec<repo::study::StudyItemRow>,
}

#[derive(Debug, serde::Serialize)]
pub struct Progress {
    pub total: i64,
    pub learned: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum StudyError {
    #[error("study topic/item 不存在: {0}")]
    NotFound(String),
    #[error("study 参数错误: {0}")]
    BadRequest(String),
    #[error("study 存储错误: {0}")]
    Storage(String),
}

impl From<engram_storage::StoreError> for StudyError {
    fn from(e: engram_storage::StoreError) -> Self {
        Self::Storage(e.to_string())
    }
}

/// 学习服务（过程状态机；知识内容归 wiki）。
#[derive(Clone)]
pub struct StudyService {
    pool: PgPool,
}

impl StudyService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 开题（新学习领域）。
    pub async fn topic_create(&self, name: &str, goal: &str) -> Result<Uuid, StudyError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StudyError::BadRequest("领域名不能为空".into()));
        }
        let id = Uuid::now_v7();
        repo::study::track_create(&self.pool, id, name, goal.trim()).await?;
        Ok(id)
    }

    /// 全量 topics（简报）。
    pub async fn topic_list(&self) -> Result<Vec<repo::study::StudyTrackRow>, StudyError> {
        Ok(repo::study::track_list(&self.pool).await?)
    }

    /// topic_get 全量：track + items + progress + next_up + in_progress。
    pub async fn topic_get(&self, id: Uuid) -> Result<Option<TopicFull>, StudyError> {
        let Some(track) = repo::study::track_get(&self.pool, id).await? else {
            return Ok(None);
        };
        let items = repo::study::items_by_track(&self.pool, id).await?;
        let (total, learned) = repo::study::track_progress(&self.pool, id).await?;
        let next_up: Vec<_> = items
            .iter()
            .filter(|i| i.status == "not_started")
            .take(5)
            .cloned()
            .collect();
        let in_progress: Vec<_> = items
            .iter()
            .filter(|i| i.status == "learning")
            .cloned()
            .collect();
        Ok(Some(TopicFull {
            track,
            items,
            progress: Progress { total, learned },
            next_up,
            in_progress,
        }))
    }

    /// 补丁式更新 topic（name/goal/status）。
    pub async fn topic_update(
        &self,
        id: Uuid,
        name: Option<&str>,
        goal: Option<&str>,
        status: Option<&str>,
    ) -> Result<(), StudyError> {
        if let Some(s) = status {
            if !matches!(s, "active" | "paused" | "done") {
                return Err(StudyError::BadRequest(format!("非法 topic status: {s}")));
            }
        }
        let n = self.exists_track(id).await?;
        if !n {
            return Err(StudyError::NotFound(format!("topic {id}")));
        }
        repo::study::track_update(&self.pool, id, name, goal, status).await?;
        Ok(())
    }

    pub async fn topic_delete(&self, id: Uuid) -> Result<(), StudyError> {
        let exists = self.exists_track(id).await?;
        if !exists {
            return Err(StudyError::NotFound(format!("topic {id}")));
        }
        repo::study::track_delete(&self.pool, id).await?;
        Ok(())
    }

    /// 加知识点（position 缺省排尾部）。
    pub async fn item_add(
        &self,
        track_id: Uuid,
        name: &str,
        position: Option<i32>,
    ) -> Result<Uuid, StudyError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StudyError::BadRequest("知识点名不能为空".into()));
        }
        if !self.exists_track(track_id).await? {
            return Err(StudyError::NotFound(format!("topic {track_id}")));
        }
        let pos = match position {
            Some(p) => p,
            None => {
                // 尾部：现有最大 position + 10
                let items = repo::study::items_by_track(&self.pool, track_id).await?;
                items.last().map(|i| i.position + 10).unwrap_or(10)
            }
        };
        let id = Uuid::now_v7();
        repo::study::item_create(&self.pool, id, track_id, name, pos).await?;
        Ok(id)
    }

    /// 知识点状态机（not_started→learning→learned；允许回退——学习本就反复）。
    pub async fn item_set_status(&self, item_id: Uuid, status: &str) -> Result<(), StudyError> {
        if !matches!(status, "not_started" | "learning" | "learned") {
            return Err(StudyError::BadRequest(format!("非法 unit status: {status}")));
        }
        let exists = repo::study::item_get(&self.pool, item_id).await?.is_some();
        if !exists {
            return Err(StudyError::NotFound(format!("item {item_id}")));
        }
        repo::study::item_update(&self.pool, item_id, None, Some(status), None, None, None)
            .await?;
        Ok(())
    }

    /// 知识点挂资料（wiki 页 slug / 文档 id）。
    pub async fn item_link(
        &self,
        item_id: Uuid,
        wiki_slugs: Option<Vec<String>>,
        doc_ids: Option<Vec<String>>,
    ) -> Result<(), StudyError> {
        let exists = repo::study::item_get(&self.pool, item_id).await?.is_some();
        if !exists {
            return Err(StudyError::NotFound(format!("item {item_id}")));
        }
        let slugs_v = wiki_slugs.map(|v| serde_json::json!(v));
        let docs_v = doc_ids.map(|v| serde_json::json!(v));
        repo::study::item_update(
            &self.pool,
            item_id,
            None,
            None,
            None,
            slugs_v.as_ref(),
            docs_v.as_ref(),
        )
        .await?;
        Ok(())
    }

    async fn exists_track(&self, id: Uuid) -> Result<bool, StudyError> {
        Ok(repo::study::track_get(&self.pool, id).await?.is_some())
    }
}
