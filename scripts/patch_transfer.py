# -*- coding: utf-8 -*-
p = 'server/crates/storage/src/repo/transfer.rs'
s = open(p, encoding='utf-8').read()

old = (
    "pub async fn import_wiki_page(pool: &PgPool, v: &Value) -> StoreResult<bool> {\n"
    "    let content = str_of(v, \"content\", \"\");\n"
    "    let res = sqlx::query(\n"
    "        \"INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, folder, tsv, created_at, updated_at) \\\n"
    "         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, to_tsvector('simple', $5), $10, $11) ON CONFLICT (slug) DO NOTHING\",\n"
    "    )\n"
    "    .bind(id_of(v, \"id\"))"
)
new = (
    "pub async fn import_wiki_page(pool: &PgPool, v: &Value) -> StoreResult<bool> {\n"
    "    let content = str_of(v, \"content\", \"\");\n"
    "    // 多库（2026-09-08）：迁移导入统一落主库（slug 冲突按库内判定）\n"
    "    let lib: Uuid = sqlx::query_scalar(\"SELECT id FROM wiki_libraries WHERE slug = 'main'\")\n"
    "        .fetch_one(pool)\n"
    "        .await?;\n"
    "    let res = sqlx::query(\n"
    "        \"INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, folder, tsv, created_at, updated_at) \\\n"
    "         VALUES ($1, $12, $2, $3, $4, $5, $6, $7, $8, $9, to_tsvector('simple', $5), $10, $11) ON CONFLICT (library_id, slug) DO NOTHING\",\n"
    "    )\n"
    "    .bind(id_of(v, \"id\"))\n"
    "    .bind(lib)"
)
assert old in s, 'transfer anchor'
s = s.replace(old, new, 1)
open(p, 'w', encoding='utf-8', newline='').write(s)
print('transfer ok')
