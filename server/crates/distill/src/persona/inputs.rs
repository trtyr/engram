//! `persona` 的实现切片（架构治理 2026-09-21：自 persona.rs 纯搬移，零行为变化）。

use super::*;

/// 入参解析的两种结局：可继续 / 已提前收尾。
pub(crate) enum Resolved {
    Ready(Inputs),
    Done(Value),
}

pub(crate) async fn resolve_inputs(ctx: &JobContext) -> Result<Resolved, JobError> {
    let pool = ctx.pool();
    let mut scenario_ids = payload_uuids(ctx, "scenario_ids");
    let mut stale_refresh: Vec<String> = Vec::new();

    // 全量重建（收录哲学线 task-10）：素材 = 全部场景，所有非钉住分面视为 stale（强制重写提示生效）
    if payload_bool(ctx, "full_rebuild") {
        let pinned_now = load_pinned(pool).await?;
        scenario_ids = sqlx::query_scalar("SELECT id FROM scenarios ORDER BY updated_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        if scenario_ids.is_empty() {
            return Ok(Resolved::Done(json!({
                "updated": [],
                "note": "全量重建：无场景素材"
            })));
        }
        stale_refresh = ASPECTS
            .iter()
            .map(|s| s.to_string())
            .filter(|a| !pinned_now.contains(a))
            .collect();
        tracing::info!(
            n = scenario_ids.len(),
            "画像全量重建：全部场景重算所有非钉住分面"
        );
    }

    if scenario_ids.is_empty() {
        // R3 画像退休：无新素材时检查分面年龄——超 7 天未更新的分面用近期场景强制重写一次
        // （剔除过期内容：过期的相对时间/失效计划/不再成立的习惯）。说过的话比不说话更伤信任。
        let stale: Vec<String> = sqlx::query_scalar(
            "WITH latest AS (SELECT DISTINCT ON (aspect) aspect, created_at \
             FROM persona_aspects ORDER BY aspect, version DESC) \
             SELECT aspect FROM latest WHERE created_at < now() - interval '7 days'",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        if stale.is_empty() {
            return Ok(Resolved::Done(json!({"updated": []})));
        }
        tracing::info!(?stale, "画像退休：陈旧分面以近期场景重写");
        stale_refresh = stale;
        scenario_ids =
            sqlx::query_scalar("SELECT id FROM scenarios ORDER BY updated_at DESC LIMIT 30")
                .fetch_all(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
        if scenario_ids.is_empty() {
            // F3 素材全空：分面写空版本（历史不可变；UI 过滤空分面不展示）——
            // 源数据没了，画像不该继续说旧话（终极清空测试 F3 化石问题）。
            let retired = write_empty_versions(pool, &stale_refresh, "f3-empty").await?;
            tracing::info!(?retired, "画像退休：素材全空，分面写空版本");
            return Ok(Resolved::Done(json!({"retired_empty": retired})));
        }
    }

    Ok(Resolved::Ready(Inputs {
        scenario_ids,
        stale_refresh,
        removed_texts: payload_strings(ctx, "removed_texts"),
    }))
}

pub(crate) fn payload_bool(ctx: &JobContext, key: &str) -> bool {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub(crate) fn payload_uuids(ctx: &JobContext, key: &str) -> Vec<Uuid> {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn payload_strings(ctx: &JobContext, key: &str) -> Vec<String> {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) async fn load_scenarios(
    pool: &sqlx::PgPool,
    scenario_ids: &[Uuid],
) -> Result<Vec<(String, String)>, JobError> {
    sqlx::query_as("SELECT topic, summary FROM scenarios WHERE id = ANY($1)")
        .bind(scenario_ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 当前画像：每个 aspect 的最新版本（含 id，供提示展示与钉住判定）。
pub(crate) async fn load_current_aspects(
    pool: &sqlx::PgPool,
) -> Result<Vec<(String, String, String)>, JobError> {
    sqlx::query_as(
        "SELECT DISTINCT ON (aspect) aspect, content, id::text \
         FROM persona_aspects ORDER BY aspect, version DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 编辑能力：用户钉住（manually_edited=true）的分面，蒸馏输出落库前丢弃——确定性保护。
/// 钉住分面仍作上下文喂给模型（保持整体一致性），但产出不落库。
pub(crate) async fn load_pinned(pool: &sqlx::PgPool) -> Result<HashSet<String>, JobError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT aspect FROM ( \
            SELECT aspect, manually_edited, \
                   row_number() OVER (PARTITION BY aspect ORDER BY version DESC) AS rn \
            FROM persona_aspects) t \
         WHERE rn = 1 AND manually_edited",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(rows.into_iter().collect())
}
