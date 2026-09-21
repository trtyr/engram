-- 0057: 项目**场景**扩类（用户 2026-09-21 拍定）。
--
-- 背景：`projects.type` 原值域只有 dev/research，实际「什么项目都往里放」——主机/设备台账、
-- 部署中转这类**运维**与个人的事也被塞进 dev，分类失去意义（生产实测：11 个项目全是 dev）。
-- 现按 6 场景分开：开发 dev / 运维 ops / 调研 research / 学习 study / 生活 life / 创作 create。
-- 值域的事实源 = core 的 `PROJECT_TYPES` 常量（本迁移的 CHECK 与它一一对应；再加类 = 改常量 + 新迁移）。
--
-- 一、放宽 CHECK（先 DROP IF EXISTS 再 ADD，可反复执行）
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_type_check;
ALTER TABLE projects ADD CONSTRAINT projects_type_check
    CHECK (type IN ('dev', 'ops', 'research', 'study', 'life', 'create'));

-- 二、存量改判（写进迁移而非手工改：同一个人的多台实例跑同一批迁移 → 云端副本自动一致）
--     判断口径：这两件事的主体不是「写代码/做产品」，而是「管设备、通网络」→ 归运维。
--     其余 9 条保持 dev（都是代码仓库/软件产品）。幂等：条件里带 type = 'dev'。
UPDATE projects SET type = 'ops', updated_at = now()
 WHERE type = 'dev' AND name IN ('主机清单', 'openlist-relay');
