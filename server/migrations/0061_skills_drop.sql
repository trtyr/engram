-- 0061: skills 域存储层全清（EN-252 裁撤的终局收尾，2026-09-26 用户拍板：前端/后端/数据库全部清空）。
-- 技能触发已回归调用方本地目录；方法论沉淀 wiki skill-* 页；engram 口径迁 projects [skills 迁入] 篇。
-- 表依赖序：skill_files → skills（0031/0032 建），skill_revisions 同批。

DROP TABLE IF EXISTS skill_files;
DROP TABLE IF EXISTS skill_revisions;
DROP TABLE IF EXISTS skills;

-- 存量 API key 的 scopes 数组清洗（scopes 是 jsonb 数组，`-` 按值删元素）：skills scope 已从 SCOPES 摘除，别名单数 skill 一并清
UPDATE api_keys SET scopes = scopes - 'skills' WHERE scopes @> '["skills"]';
UPDATE api_keys SET scopes = scopes - 'skill' WHERE scopes @> '["skill"]';
