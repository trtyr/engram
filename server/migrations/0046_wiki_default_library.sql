-- 0046: wiki 默认库 main 的幂等对账（EN-57）
-- 0037 在多库拆分当时用 WHERE NOT EXISTS 建过 main；但那只保护「迁移 0037 执行时刻」。
-- 此后 main 若因误删 / 部分数据恢复（如 pg_dump --schema-only 恢复结构）而缺失，
-- 已应用的迁移不会重跑，系统从此每次 wiki 操作都报「库 main 不存在」且无自愈。
-- 本迁移对所有升级到 46 版的库做一次 main 存在性对账：
--   已有 main 的库零影响（ON CONFLICT (slug) DO NOTHING 跳过，id/name 不动）；
--   缺 main 的库补建（固定保留段 UUID——跨环境稳定、永不与运行时 v7 撞车）。
INSERT INTO wiki_libraries (id, slug, name)
VALUES ('00000000-0000-4000-8000-000000000001', 'main', '个人知识库')
ON CONFLICT (slug) DO NOTHING;
