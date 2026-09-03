-- 0025: Wiki — 页面 folder 字段（Obsidian 式目录树层级，/ 分隔多级路径）。
-- 蒸馏生成页面默认按 page_type 归文件夹（见 crate::service::folder_for_type），
-- 用户可手动改（PUT /wiki/pages/{slug} 带 folder 字段）。
ALTER TABLE wiki_pages ADD COLUMN folder text NOT NULL DEFAULT '';
