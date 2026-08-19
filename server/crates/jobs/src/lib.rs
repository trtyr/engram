//! 任务系统：PG-backed 队列（SKIP LOCKED 抢占）、重试、链式入队、事件流。
//! 一切长操作（蒸馏/摄取/ingest/同步）都走 job（Phase 1 完整实现）。
