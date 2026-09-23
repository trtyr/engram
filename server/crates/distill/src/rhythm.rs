//! 内置节律（内置节律线 roadmap v3）：节律任务用 jobs 基建自续，外部 crontab 与
//! 心跳通道整体退役（2026-09-18 用户拍板；单用户自托管不留过渡期）。
//!
//! 三条铁律：
//! 1. **先续期再干活**——与 cron 语义一致（本轮失败不影响下一期），任务失败/重试/
//!    dead 都不会让节律停摆；若先干活后续期，一次永久失败就会断链。
//! 2. **周期桶幂等键全局唯一**——`rhythm-extract-{slot}` / `rhythm-consolidate-{日期}-{时}`，
//!    enqueue 同键命中即复用既有任务（任何状态），启动补建 / 并发续期 / 崩溃恢复天然去重。
//! 3. **两个 handler 都续整张节律表**——互为自愈：任一任务运行都会把另一条的下一期补上。
//!
//! cron 语义对齐现状（原安装向导口径）：周期增量蒸馏（rhythm_extract）+ 每日全量整理
//! （rhythm_consolidate，含 consolidate 全量阶段）。周期配置存 settings 表 `rhythm` 键。

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use engram_jobs::types::{JobError, JobTemplate};
use engram_jobs::{JobQueue, Runner};
use sqlx::PgPool;

use crate::llm_port::LlmRef;

/// settings 表键：节律配置。
pub const SETTINGS_KEY: &str = "rhythm";

/// 节律任务种类（jobs.kind）。
pub const KIND_EXTRACT: &str = "rhythm_extract";
pub const KIND_CONSOLIDATE: &str = "rhythm_consolidate";

/// 节律配置（settings.rhythm；缺行/坏 JSON 按缺省处理，不让配置损坏打死节律）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RhythmConfig {
    /// 总开关：停用后不再续期，已排的下一期跑完即自然停止。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 增量蒸馏周期（小时，1-720）。
    #[serde(default = "default_extract_hours")]
    pub extract_every_hours: i64,
    /// 每日全量整理触发小时（服务器本地时区 0-23）。
    #[serde(default = "default_consolidate_hour")]
    pub consolidate_hour_local: u32,
}

fn default_true() -> bool {
    true
}
fn default_extract_hours() -> i64 {
    6
}
fn default_consolidate_hour() -> u32 {
    3
}

impl Default for RhythmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            extract_every_hours: 6,
            consolidate_hour_local: 3,
        }
    }
}

/// 读配置（行缺失/坏 JSON → 缺省）。
pub async fn load_config(pool: &PgPool) -> RhythmConfig {
    engram_storage::repo::settings::get_json(pool, SETTINGS_KEY)
        .await
        .unwrap_or_default()
}

/// 写配置（设置页 PUT 用；调用方负责合法性校验）。
pub async fn save_config(
    pool: &PgPool,
    cfg: &RhythmConfig,
) -> engram_storage::error::StoreResult<()> {
    engram_storage::repo::settings::put_json(pool, SETTINGS_KEY, cfg).await
}

/// 下一期增量蒸馏模板（纯函数，单测锁定）：
/// slot = floor(now / 周期) + 1，due_at = slot × 周期——任务在其所属周期的边界时刻到期；
/// 键含 slot 全局唯一，跨周期推进永不重键。
pub fn next_extract_template(now: DateTime<Utc>, every_hours: i64) -> JobTemplate {
    let period = every_hours.clamp(1, 24 * 30) * 3600;
    let slot = now.timestamp().div_euclid(period) + 1;
    let due = Utc.timestamp_opt(slot * period, 0).single().unwrap_or(now);
    JobTemplate::new(KIND_EXTRACT)
        .with_idempotency_key(format!("rhythm-extract-{slot}"))
        .with_payload(serde_json::json!({"reason": "rhythm"}))
        .with_due(due)
}

/// 下一期每日全量整理模板（纯函数，单测锁定）：
/// 下一个「本地 hour 点整」触发；键 = 触发日 + 小时（改配置即时生效——新键不会被
/// 旧同期成功任务的去重挡住）。DST 歧义取较早解，无效时刻回退 UTC 口径。
pub fn next_consolidate_template(
    now: DateTime<Utc>,
    hour_local: u32,
    offset: FixedOffset,
) -> JobTemplate {
    let hour = hour_local.clamp(0, 23);
    let now_local = now.with_timezone(&offset);
    let today = now_local.date_naive();
    let at = |d: chrono::NaiveDate| {
        let naive = d.and_hms_opt(hour, 0, 0).unwrap_or_default();
        match offset.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) => Some(dt),
            chrono::LocalResult::Ambiguous(a, _) => Some(a),
            chrono::LocalResult::None => Some(dt_utc_fallback(d, hour, offset)),
        }
    };
    let (due_date, due_local) = match at(today) {
        Some(dt) if dt > now_local => (today, dt),
        _ => {
            let tomorrow = today.succ_opt().unwrap_or(today);
            let dt = at(tomorrow).unwrap_or_else(|| dt_utc_fallback(tomorrow, hour, offset));
            (tomorrow, dt)
        }
    };
    let key = due_date.format("%Y%m%d");
    JobTemplate::new(KIND_CONSOLIDATE)
        .with_idempotency_key(format!("rhythm-consolidate-{key}-{hour:02}"))
        .with_payload(serde_json::json!({"reason": "rhythm"}))
        .with_due(due_local.with_timezone(&Utc))
}

/// DST 无效时刻（春季跳变）的兜底：按 UTC 同钟点折算，宁可偏移也不缺席。
fn dt_utc_fallback(d: chrono::NaiveDate, hour: u32, offset: FixedOffset) -> DateTime<FixedOffset> {
    let naive_utc = d.and_hms_opt(hour, 0, 0).unwrap_or_default();
    Utc.from_utc_datetime(&naive_utc).with_timezone(&offset)
}

/// 两期任务模板一次算好（extract + consolidate）。
pub fn next_templates(
    now: DateTime<Utc>,
    cfg: &RhythmConfig,
    offset: FixedOffset,
) -> Vec<JobTemplate> {
    vec![
        next_extract_template(now, cfg.extract_every_hours),
        next_consolidate_template(now, cfg.consolidate_hour_local, offset),
    ]
}

/// 确保下一期待办在队（bootstrap 与任务续期共用）：幂等键命中即复用，绝不重复投递。
/// 并发撞唯一约束（idempotency_conflict）视为成功——对方已投。
pub async fn schedule_next(
    queue: &JobQueue,
    cfg: &RhythmConfig,
) -> Result<Vec<engram_jobs::Job>, JobError> {
    if !cfg.enabled {
        return Ok(Vec::new());
    }
    let offset = local_offset();
    let mut out = Vec::with_capacity(2);
    for t in next_templates(Utc::now(), cfg, offset) {
        match queue.enqueue(t).await {
            Ok(job) => out.push(job),
            Err(JobError::Permanent(e)) if e.contains("idempotency_conflict") => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// server 启动自检补建（重复启动/崩溃恢复安全）：读配置 → enabled 则确保下一期在队。
/// 返回在队任务数（含键命中复用的既有待办）。
pub async fn bootstrap(queue: &JobQueue, pool: &PgPool) -> Result<usize, JobError> {
    let cfg = load_config(pool).await;
    let jobs = schedule_next(queue, &cfg).await?;
    if jobs.is_empty() {
        tracing::info!("内置节律：已停用（settings.rhythm.enabled=false）");
    } else {
        let extract_due = jobs
            .iter()
            .find(|j| j.kind == KIND_EXTRACT)
            .map(|j| j.due_at.to_rfc3339())
            .unwrap_or_default();
        let consolidate_due = jobs
            .iter()
            .find(|j| j.kind == KIND_CONSOLIDATE)
            .map(|j| j.due_at.to_rfc3339())
            .unwrap_or_default();
        tracing::info!(
            extract_due = %extract_due,
            consolidate_due = %consolidate_due,
            "内置节律：下一期待办已确保在队",
        );
    }
    Ok(jobs.len())
}

/// 注册节律 handler（main 装配用）。
///
/// **先续期再干活**：enqueue 幂等键命中即复用，绝不重复投递；本轮失败/重试不影响
/// 下一期（cron 语义）。停用时跳过续期与工作（已排的下一期跑完即自然停）。
pub fn register_rhythm(runner: Runner, llm: LlmRef) -> Runner {
    let l_extract = llm.clone();
    let l_consolidate = llm;
    runner
        .register(KIND_EXTRACT, move |ctx| {
            let llm = l_extract.clone();
            async move {
                let cfg = load_config(ctx.pool()).await;
                if !cfg.enabled {
                    return Ok(serde_json::json!({"skipped": "rhythm_disabled"}));
                }
                for t in next_templates(Utc::now(), &cfg, local_offset()) {
                    ctx.enqueue_next(t).await?;
                }
                crate::extract::run(ctx, llm).await
            }
        })
        .register(KIND_CONSOLIDATE, move |ctx| {
            let llm = l_consolidate.clone();
            async move {
                let cfg = load_config(ctx.pool()).await;
                if !cfg.enabled {
                    return Ok(serde_json::json!({"skipped": "rhythm_disabled"}));
                }
                for t in next_templates(Utc::now(), &cfg, local_offset()) {
                    ctx.enqueue_next(t).await?;
                }
                crate::consolidate::run(ctx, llm).await
            }
        })
}

fn local_offset() -> FixedOffset {
    *chrono::Local::now().offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// 东八区固定偏移（不依赖机器时区，测试可复现）。
    fn utc8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }
    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().unwrap()
    }

    #[test]
    fn extract_slot_next_period_and_unique_key() {
        let period = 6 * 3600;
        // 周期中段任意时刻：下一期 = 当前桶 + 1，到期 = 桶边界
        let now = ts(1_700_000_123);
        let t = next_extract_template(now, 6);
        let slot = now.timestamp().div_euclid(period) + 1;
        assert_eq!(
            t.idempotency_key.as_deref(),
            Some(format!("rhythm-extract-{slot}").as_str())
        );
        assert_eq!(t.due_at.unwrap().timestamp(), slot * period);
        // 周期推进后键必然不同（自续链不重不漏）
        let t2 = next_extract_template(ts(slot * period + 1), 6);
        assert_ne!(t.idempotency_key, t2.idempotency_key);
    }

    #[test]
    fn extract_boundary_and_clamp() {
        // 恰在边界：floor(边界)=新桶，下一期 = 新桶+1，到期 = slot×周期
        let t = next_extract_template(ts(36_000), 6); // 10h = 1.67 周期 → 桶 1 → slot 2
        assert_eq!(t.due_at.unwrap().timestamp(), 2 * 21_600);
        // 非法周期（0/负）按 1 小时处理：slot = 0+1 = 1，到期 = 3600
        let t0 = next_extract_template(ts(100), 0);
        assert_eq!(t0.due_at.unwrap().timestamp(), 3_600);
        // 超大周期封顶 720h：slot = 0+1 = 1，到期 = 720×3600
        let t720 = next_extract_template(ts(100), 100_000);
        assert_eq!(t720.due_at.unwrap().timestamp(), 720 * 3_600);
    }

    #[test]
    fn consolidate_before_hour_today_after_tomorrow() {
        let off = utc8();
        let day = NaiveDate::from_ymd_opt(2025, 9, 23).unwrap();
        let now = off
            .from_local_datetime(&day.and_hms_opt(10, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc); // 10:00+08

        // hour=3 已过 → 明日 03:00+08，键含明日日期
        let tomorrow = day.succ_opt().unwrap();
        let t = next_consolidate_template(now, 3, off);
        assert_eq!(
            t.idempotency_key.as_deref(),
            Some(format!("rhythm-consolidate-{}-03", tomorrow.format("%Y%m%d")).as_str())
        );
        let due_local = t.due_at.unwrap().with_timezone(&off);
        assert_eq!(due_local.date_naive(), tomorrow);
        assert_eq!(due_local.format("%H").to_string(), "03");

        // hour=23 未到 → 今天 23:00+08
        let t2 = next_consolidate_template(now, 23, off);
        assert_eq!(
            t2.idempotency_key.as_deref(),
            Some(format!("rhythm-consolidate-{}-23", day.format("%Y%m%d")).as_str())
        );
        assert_eq!(t2.due_at.unwrap().with_timezone(&off).date_naive(), day);

        // 恰在整点：now == 今日该点（不严格大于）→ 明日（同一时刻不重复触发）
        let at3 = off
            .from_local_datetime(&day.and_hms_opt(3, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc);
        let t3 = next_consolidate_template(at3, 3, off);
        assert_eq!(
            t3.idempotency_key.as_deref(),
            Some(format!("rhythm-consolidate-{}-03", tomorrow.format("%Y%m%d")).as_str())
        );
    }

    #[test]
    fn consolidate_month_and_year_rollover() {
        let off = utc8();
        let dec31 = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        let now = off
            .from_local_datetime(&dec31.and_hms_opt(23, 0, 0).unwrap())
            .unwrap()
            .with_timezone(&Utc);
        let t = next_consolidate_template(now, 3, off);
        assert_eq!(
            t.idempotency_key.as_deref(),
            Some("rhythm-consolidate-20260101-03")
        );
    }

    #[test]
    fn config_defaults_survive_garbage() {
        let cfg: RhythmConfig = serde_json::from_str("not json").unwrap_or_default();
        assert!(cfg.enabled);
        assert_eq!(cfg.extract_every_hours, 6);
        assert_eq!(cfg.consolidate_hour_local, 3);
        // 部分字段缺失 → 缺省补齐
        let partial: RhythmConfig =
            serde_json::from_str(r#"{"enabled": false}"#).unwrap_or_default();
        assert!(!partial.enabled);
        assert_eq!(partial.extract_every_hours, 6);
    }
}
