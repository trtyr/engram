# -*- coding: utf-8 -*-
"""core wiki_docs service lib-first + unified per-lib search + main backfill。"""

# 1) storage repo: document_library
p = 'server/crates/storage/src/repo/wiki_docs.rs'
s = open(p, encoding='utf-8').read()
if 'pub async fn document_library' not in s:
    s += '''
/// 文档所属库（摄取 job 链回查 library 用；文档不存在返回 None）。
pub async fn document_library(pool: &PgPool, doc_id: Uuid) -> StoreResult<Option<Uuid>> {
    sqlx::query_scalar("SELECT library_id FROM wiki_documents WHERE id = $1")
        .bind(doc_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}
'''
    open(p, 'w', encoding='utf-8', newline='').write(s)
    print('repo ok')
else:
    print('repo already')

# 2) core wiki_docs mod.rs：service 方法 lib-first
p = 'server/crates/core/src/wiki_docs/mod.rs'
s = open(p, encoding='utf-8').read()
subs = [
    ("    pub async fn submit(&self, source: IngestSource) -> Result<(Uuid, bool), WikiDocumentError> {",
     "    pub async fn submit(&self, lib: Uuid, source: IngestSource) -> Result<(Uuid, bool), WikiDocumentError> {"),
    ("        pipeline::enqueue_ingest(&self.queue, &self.registry, &self.data_dir, source).await",
     "        pipeline::enqueue_ingest(&self.queue, &self.registry, &self.data_dir, lib, source).await"),
    ("    pub async fn list_documents(\n        &self,\n        status: Option<&str>,",
     "    pub async fn list_documents(\n        &self,\n        lib: Uuid,\n        status: Option<&str>,"),
    ("        repo::list_documents(&self.pool, status, cursor, limit.min(200))",
     "        repo::list_documents(&self.pool, lib, status, cursor, limit.min(200))"),
    ("    pub async fn get_document(&self, id: Uuid) -> Result<DocumentDto, WikiDocumentError> {\n        repo::get_document(&self.pool, id)",
     "    pub async fn get_document(&self, lib: Uuid, id: Uuid) -> Result<DocumentDto, WikiDocumentError> {\n        repo::get_document(&self.pool, lib, id)"),
    ("    pub async fn chunks(\n        &self,\n        id: Uuid,\n        limit: i64,\n    ) -> Result<Vec<(i32, String, bool)>, WikiDocumentError> {\n        repo::list_chunks(&self.pool, id, limit)",
     "    pub async fn chunks(\n        &self,\n        lib: Uuid,\n        id: Uuid,\n        limit: i64,\n    ) -> Result<Vec<(i32, String, bool)>, WikiDocumentError> {\n        repo::list_chunks(&self.pool, lib, id, limit)"),
    ("    pub async fn delete(&self, id: Uuid) -> Result<(), WikiDocumentError> {\n        let raw_path = repo::delete_document_returning_path(&self.pool, id)",
     "    pub async fn delete(&self, lib: Uuid, id: Uuid) -> Result<(), WikiDocumentError> {\n        let raw_path = repo::delete_document_returning_path(&self.pool, lib, id)"),
    ("    pub async fn reembed(&self, id: Uuid) -> Result<(), WikiDocumentError> {\n        let status = repo::get_document_status(&self.pool, id)",
     "    pub async fn reembed(&self, lib: Uuid, id: Uuid) -> Result<(), WikiDocumentError> {\n        let status = repo::get_document_status(&self.pool, lib, id)"),
    ("    pub async fn search(\n        &self,\n        query: &str,\n        limit: i64,\n    ) -> Result<Vec<ChunkHit>, WikiDocumentError> {",
     "    pub async fn search(\n        &self,\n        lib: Uuid,\n        query: &str,\n        limit: i64,\n    ) -> Result<Vec<ChunkHit>, WikiDocumentError> {"),
    ("        let rows = repo::search_chunks(&self.pool, &tsv_query_smart(query, 3), qv, limit)",
     "        let rows = repo::search_chunks(&self.pool, lib, &tsv_query_smart(query, 3), qv, limit)"),
]
for i, (old, new) in enumerate(subs, 1):
    assert old in s, 'mod MISS %d: %s' % (i, old[:60])
    s = s.replace(old, new, 1)
open(p, 'w', encoding='utf-8', newline='').write(s)
print('mod ok')

# 3) unified.rs：跨库检索（每库各查后并集，RRF 归一化不变）
p = 'server/crates/core/src/unified.rs'
s = open(p, encoding='utf-8').read()
old = """        // 三域并行检索 + 实体层（各自降级：无 embedding 时退化为 FTS，不互相阻塞）
        let (mem_res, know_res, wiki_res, ent_res, todo_res) = tokio::join!(
            mem.search(query, &["l1", "l2"], per_domain, true, false, None, None),
            know.search(query, per_domain),
            wiki.search(query, per_domain),"""
new = """        // 多库（2026-09-08）：文档与 wiki 页按库各查一份再并集（RRF 归一化不变）
        let lib_ids: Vec<Uuid> = crate::wiki::libraries::list(&self.pool)
            .await
            .iter()
            .map(|l| l.id)
            .collect();

        // 三域并行检索 + 实体层（各自降级：无 embedding 时退化为 FTS，不互相阻塞）
        let (mem_res, know_res, wiki_res, ent_res, todo_res) = tokio::join!(
            mem.search(query, &["l1", "l2"], per_domain, true, false, None, None),
            async {
                let mut out = Vec::new();
                for lib in &lib_ids {
                    if let Ok(mut hits) = know.search(*lib, query, per_domain).await {
                        out.append(&mut hits);
                    }
                }
                Ok::<_, UnifiedError>(out)
            },
            async {
                let mut out = Vec::new();
                for lib in &lib_ids {
                    if let Ok(mut hits) = wiki.search(*lib, query, per_domain).await {
                        out.append(&mut hits);
                    }
                }
                Ok::<_, UnifiedError>(out)
            },"""
assert old in s, 'unified join'
s = s.replace(old, new, 1)
open(p, 'w', encoding='utf-8', newline='').write(s)
print('unified ok')

# 4) main.rs：tsv 回填按库
p = 'server/crates/api/src/main.rs'
s = open(p, encoding='utf-8').read()
old = """            match wiki.backfill_tsv().await {
                Ok(n) if n > 0 => tracing::info!("wiki tsv 存量补数完成：{n} 页"),
                Ok(_) => {}
                Err(e) => tracing::warn!("wiki tsv 存量补数失败（下次启动重试）: {e}"),
            }"""
new = """            for lib in engram_core::wiki::libraries::list(&st.pool).await {
                match wiki.backfill_tsv(lib.id).await {
                    Ok(n) if n > 0 => tracing::info!("wiki tsv 存量补数完成：{}（{}）{n} 页", lib.slug, lib.name),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("wiki tsv 存量补数失败（{} 下次启动重试）: {e}", lib.slug),
                }
            }"""
assert old in s, 'main backfill'
s = s.replace(old, new, 1)
open(p, 'w', encoding='utf-8', newline='').write(s)
print('main ok')
