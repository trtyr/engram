-- 0050: 项目文档乐观锁（公网多Agent P001 步骤2，2026-09-17）
-- version：每次 doc_update/patch 成功 +1；写入方可带 expected_version 做陈旧检测（409）。
ALTER TABLE project_docs ADD COLUMN version bigint NOT NULL DEFAULT 1;
