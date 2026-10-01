//! 结构化日志落地（P005-T001）：tracing 事件 → PG `logs` 表。
//!
//! 架构：`PgLogLayer`（tracing Layer）把过滤后的事件经 **mpsc channel** 送出——
//! 层内零 IO，请求路径不阻塞；`spawn_log_writer` 在 pool 就绪后启动，后台
//! 批量 INSERT（攒满 64 条或 500ms 先到者 flush）。`spawn_logs_retention`
//! 每小时清理过期行（info 级 30 天 / debug 级 7 天，可 env 覆盖）。
//!
//! 红线：凭据值/token 永不写入 fields（凭据纪律——写侧不做 secrets 过滤，
//! 依赖各业务域「凭据值不入日志」的既有纪律）。

use sqlx::PgPool;
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const CHANNEL_CAP: usize = 8192;
const BATCH_SIZE: usize = 64;
const FLUSH_INTERVAL_MS: u64 = 500;

/// 单条待写日志（layer → writer 的消息）。
#[derive(Debug)]
pub struct LogRecord {
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: serde_json::Value,
}

/// tracing Layer：事件序列化进 channel。
pub struct PgLogLayer {
    tx: mpsc::Sender<LogRecord>,
}

struct FieldVisitor {
    message: Option<String>,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::json!(format!("{value:?}")),
            );
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .insert(field.name().to_string(), serde_json::json!(value));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), serde_json::json!(value));
    }
}

tokio::task_local! {
    /// 当前 task 的 request-id（request_id_mw 设置——HTTP 范围内全部日志自动关联）。
    static REQUEST_ID: String;
}

/// 在 request-id 上下文内执行（测试与工具用——mw 内部自动设置）。
pub async fn with_request_id<T>(rid: String, fut: impl std::future::Future<Output = T>) -> T {
    REQUEST_ID.scope(rid, fut).await
}

/// 当前 task 的 request-id（HTTP 范围内 Some；错误信封注入用）。
pub fn current_request_id() -> Option<String> {
    REQUEST_ID.try_with(|v| v.clone()).ok()
}

impl<S> Layer<S> for PgLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor {
            message: None,
            fields: serde_json::Map::new(),
        };
        event.record(&mut visitor);
        let level = event.metadata().level().to_string();
        let target = event.metadata().target().to_string();
        let message = visitor.message.unwrap_or_else(|| {
            visitor
                .fields
                .remove("message")
                .map(|v| v.to_string())
                .unwrap_or_default()
        });
        // task_local 贯穿：HTTP handler task 内的所有日志自动带 request_id
        if let Ok(rid) = REQUEST_ID.try_with(|v| v.clone()) {
            visitor
                .fields
                .insert("request_id".into(), serde_json::json!(rid));
        }
        // P006-T003：internal_bug 类（EngramError 归因）自动升级告警标记——必须人工跟进
        if visitor.fields.get("category").and_then(|v| v.as_str()) == Some("internal_bug") {
            visitor
                .fields
                .insert("alert".into(), serde_json::json!(true));
        }
        // channel 满即丢弃（logging 永不反压业务路径）
        let _ = self.tx.try_send(LogRecord {
            level,
            target,
            message,
            fields: serde_json::Value::Object(visitor.fields),
        });
    }
}

/// x-request-id 贯穿 + HTTP 请求日志（P005-T002）。
/// 无入站头则生成 uuid7；响应头回带；请求日志（方法/路由模板/状态/耗时）落 logs。
pub async fn request_id_mw(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let rid = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string());
    let method = req.method().clone();
    let path = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let start = std::time::Instant::now();

    let serve = REQUEST_ID.scope(rid.clone(), async {
        let res = next.run(req).await;
        let latency_ms = start.elapsed().as_millis() as u64;
        let status = res.status().as_u16();
        tracing::info!(
            request_id = %rid,
            http_method = %method,
            http_path = %path,
            http_status = status,
            latency_ms,
            "http 请求"
        );
        res
    });
    let mut res = serve.await;
    if let Ok(v) = rid.parse() {
        res.headers_mut().insert("x-request-id", v);
    }
    res
}

/// 建 layer + 接收端。pool 就绪后把 rx 交给 [`spawn_log_writer`]。
pub fn channel() -> (PgLogLayer, mpsc::Receiver<LogRecord>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAP);
    (PgLogLayer { tx }, rx)
}

/// 后台写者：批量攒写 + 定时 flush；channel 关闭（shutdown）即退出。
/// 返回 keepalive（writer 任务句柄）。
pub fn spawn_log_writer(
    pool: PgPool,
    mut rx: mpsc::Receiver<LogRecord>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut batch: Vec<LogRecord> = Vec::with_capacity(BATCH_SIZE);
        let flush_tick = tokio::time::interval(std::time::Duration::from_millis(FLUSH_INTERVAL_MS));
        tokio::pin!(flush_tick);
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(rec) => {
                            batch.push(rec);
                            // 攒满即 flush（不满等 tick——500ms 内的小批量合并成一次 INSERT）
                            if batch.len() >= BATCH_SIZE {
                                flush(&pool, &mut batch).await;
                            }
                        }
                        None => break, // 所有 sender drop：shutdown
                    }
                }
                _ = flush_tick.tick() => {
                    flush(&pool, &mut batch).await; // 空批无害跳过
                }
            }
        }
        // 收尾：channel 关闭后清空余量
        if !batch.is_empty() {
            flush(&pool, &mut batch).await;
        }
    })
}

/// 批量 INSERT（单条多 VALUES；失败 warn 留痕不重试——日志丢批可接受，业务不可反压）。
async fn flush(pool: &PgPool, batch: &mut Vec<LogRecord>) {
    if batch.is_empty() {
        return;
    }
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO logs (level, target, message, fields, request_id) ",
    );
    qb.push_values(batch.iter(), |mut b, rec| {
        // fields["request_id"] 提升到列（idx_logs_request_id 索引查询面）
        let rid = rec
            .fields
            .get("request_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        b.push_bind(&rec.level)
            .push_bind(&rec.target)
            .push_bind(&rec.message)
            .push_bind(&rec.fields)
            .push_bind(rid);
    });
    if let Err(e) = qb.build().execute(pool).await {
        tracing::warn!(error = %e, dropped = batch.len(), "logs 批量写入失败");
    }
    batch.clear();
}

/// 保留期清理（单轮，可测）：info+ 保留 info_days 天、debug 级保留 debug_days 天。
/// 返回删除行数。
pub async fn retain_old(pool: &PgPool, info_days: i64, debug_days: i64) -> anyhow::Result<u64> {
    let cutoff_info = chrono::Utc::now() - chrono::Duration::days(info_days);
    let cutoff_debug = chrono::Utc::now() - chrono::Duration::days(debug_days);
    // debug 级按短保留期；其余按长保留期
    let res = sqlx::query(
        "DELETE FROM logs WHERE (level = 'DEBUG' AND ts < $1) OR (level <> 'DEBUG' AND ts < $2)",
    )
    .bind(cutoff_debug)
    .bind(cutoff_info)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 保留期清理任务：每小时跑一轮；info+ 保留 30 天、debug 级保留 7 天（env 可覆盖天数）。
pub fn spawn_logs_retention(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let info_days: i64 = std::env::var("ENGRAM_LOG_RETAIN_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let debug_days: i64 = std::env::var("ENGRAM_LOG_RETAIN_DEBUG_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        tick.tick().await; // 首跳立即消费（启动不清理）
        loop {
            tick.tick().await;
            if let Err(e) = retain_old(&pool, info_days, debug_days).await {
                tracing::warn!(error = %e, "logs 保留期清理失败");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    /// channel 层：事件 → LogRecord（message/fields 拆分、channel 满不反压）。
    #[test]
    fn layer_forwards_records_via_channel() {
        let (layer, mut rx) = channel();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(doc = "测试文档", "文档摄取失败");
            tracing::warn!(count = 42, "批次降级");
        });
        let r1 = rx.blocking_recv().expect("第一条");
        assert_eq!(r1.level, "INFO");
        assert_eq!(r1.message, "文档摄取失败");
        assert_eq!(r1.fields["doc"], "测试文档");
        let r2 = rx.blocking_recv().expect("第二条");
        assert_eq!(r2.level, "WARN");
        assert_eq!(r2.fields["count"], 42);
    }

    /// P006-T003：internal_bug 类事件自动升级告警标记。
    #[test]
    fn internal_bug_events_get_alert_flag() {
        let (layer, mut rx) = channel();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(
                category = "internal_bug",
                code = "INTERNAL-INCONSISTENT",
                source = "孤儿态: job dead + 文档 pending",
                "内部状态不一致"
            );
            tracing::error!(category = "upstream", "普通上游错误");
        });
        let r1 = rx.blocking_recv().expect("internal_bug 事件");
        assert_eq!(r1.fields["alert"], true, "internal_bug 应升级告警");
        assert_eq!(r1.fields["code"], "INTERNAL-INCONSISTENT");
        assert_eq!(r1.fields["source"], "孤儿态: job dead + 文档 pending");
        let r2 = rx.blocking_recv().expect("普通错误事件");
        assert!(
            r2.fields.get("alert").is_none(),
            "非 internal_bug 不应有告警标记"
        );
    }
}
