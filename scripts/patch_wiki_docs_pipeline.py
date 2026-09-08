# -*- coding: utf-8 -*-
"""wiki_docs pipeline 多库穿参一次性补丁。"""
p = 'server/crates/core/src/wiki_docs/pipeline.rs'
s = open(p, encoding='utf-8').read()

subs = []

subs.append((
    "pub async fn enqueue_ingest(\n    queue: &engram_jobs::JobQueue,\n    _registry: &ProviderRegistry,\n    data_dir: &std::path::Path,\n    source: IngestSource,\n)",
    "pub async fn enqueue_ingest(\n    queue: &engram_jobs::JobQueue,\n    _registry: &ProviderRegistry,\n    data_dir: &std::path::Path,\n    lib: Uuid,\n    source: IngestSource,\n)",
))

subs.append((
    "    let inserted = repo::insert_document_sha(\n        queue.pool(),\n        id,",
    "    let inserted = repo::insert_document_sha(\n        queue.pool(),\n        lib,\n        id,",
))

subs.append((
    "        let existing = repo::find_document_id_by_sha(queue.pool(), &sha)",
    "        let existing = repo::find_document_id_by_sha(queue.pool(), lib, &sha)",
))

subs.append((
    "        let healed = repo::heal_stuck_document(queue.pool(), existing)",
    "        let healed = repo::heal_stuck_document(queue.pool(), lib, existing)",
))

subs.append((
    '                    JobTemplate::new("parse_document")\n                        .with_payload(json!({"document_id": existing}))',
    '                    JobTemplate::new("parse_document")\n                        .with_payload(json!({"document_id": existing, "library_id": lib}))',
))

subs.append((
    '            JobTemplate::new("parse_document")\n                .with_payload(json!({"document_id": id}))',
    '            JobTemplate::new("parse_document")\n                .with_payload(json!({"document_id": id, "library_id": lib}))',
))

subs.append((
    "        let _ = repo::delete_document_quiet(queue.pool(), id).await;",
    "        let _ = repo::delete_document_quiet(queue.pool(), lib, id).await;",
))

subs.append((
    "    // 读取文档行（raw_path 可空：URL 文档摄取前无本地文件）\n    let (source_uri, raw_path, mime) = repo::get_document_source(pool, doc_id)",
    "    // 文档所属库（多库：全部 repo 调用按库收窄）\n    let lib: Uuid = repo::document_library(pool, doc_id)\n        .await\n        .map_err(|e| JobError::Retryable(e.to_string()))?\n        .ok_or_else(|| JobError::Permanent(format!(\"文档 {doc_id} 不存在\")))?;\n    // 读取文档行（raw_path 可空：URL 文档摄取前无本地文件）\n    let (source_uri, raw_path, mime) = repo::get_document_source(pool, lib, doc_id)",
))

subs.append((
    "    async fn mark_failed(ctx: &JobContext, doc_id: Uuid, msg: &str) {\n        let _ = repo::mark_failed_document(ctx.pool(), doc_id, msg).await;",
    "    let mark_failed = |ctx: &JobContext, msg: String| {\n        let _ = repo::mark_failed_document(ctx.pool(), lib, doc_id, &msg).await;",
))

subs.append((
    "                let _ = repo::update_document_fetch_result(\n                    pool,\n                    doc_id,",
    "                let _ = repo::update_document_fetch_result(\n                    pool,\n                    lib,\n                    doc_id,",
))

subs.append((
    '    // 状态 → parsing\n    repo::update_document_status(pool, doc_id, "parsing")',
    '    // 状态 → parsing\n    repo::update_document_status(pool, lib, doc_id, "parsing")',
))

subs.append((
    '    ctx.enqueue_next(\n        JobTemplate::new("chunk_document").with_payload(json!({"document_id": doc_id})),\n    )\n    .await?;\n    Ok(json!({"document_id": doc_id, "chars": text.chars().count()}))',
    '    ctx.enqueue_next(\n        JobTemplate::new("chunk_document")\n            .with_payload(json!({"document_id": doc_id, "library_id": lib})),\n    )\n    .await?;\n    Ok(json!({"document_id": doc_id, "chars": text.chars().count()}))',
))

subs.append((
    '    repo::update_document_status(pool, doc_id, "chunking")',
    '    let lib: Uuid = repo::document_library(pool, doc_id)\n        .await\n        .map_err(|e| JobError::Retryable(e.to_string()))?\n        .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;\n    repo::update_document_status(pool, lib, doc_id, "chunking")',
))

subs.append((
    '        repo::mark_failed_document(pool, doc_id, "解析后内容为空")',
    '        repo::mark_failed_document(pool, lib, doc_id, "解析后内容为空")',
))

subs.append((
    "        repo::insert_chunk(\n            pool,\n            Uuid::now_v7(),\n            doc_id,",
    "        repo::insert_chunk(\n            pool,\n            lib,\n            Uuid::now_v7(),\n            doc_id,",
))

subs.append((
    '    ctx.enqueue_next(\n        JobTemplate::new("embed_document").with_payload(json!({"document_id": doc_id})),\n    )\n    .await?;\n    Ok(json!({"document_id": doc_id, "chunks": chunks.len()}))',
    '    ctx.enqueue_next(\n        JobTemplate::new("embed_document")\n            .with_payload(json!({"document_id": doc_id, "library_id": lib})),\n    )\n    .await?;\n    Ok(json!({"document_id": doc_id, "chunks": chunks.len()}))',
))

subs.append((
    '    repo::update_document_status(pool, doc_id, "embedding")',
    '    let lib: Uuid = repo::document_library(pool, doc_id)\n        .await\n        .map_err(|e| JobError::Retryable(e.to_string()))?\n        .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;\n    repo::update_document_status(pool, lib, doc_id, "embedding")',
))

subs.append((
    "    let chunks = repo::missing_chunks(pool, doc_id)",
    "    let chunks = repo::missing_chunks(pool, lib, doc_id)",
))

subs.append((
    "    let total = repo::count_chunks(pool, doc_id)",
    "    let total = repo::count_chunks(pool, lib, doc_id)",
))

subs.append((
    "                        for (cid, _) in batch {\n                            repo::set_chunk_failed(pool, *cid)",
    "                        for (cid, _) in batch {\n                            repo::set_chunk_failed(pool, lib, *cid)",
))

subs.append((
    "                    for (i, (cid, _)) in batch.iter().enumerate() {\n                        repo::set_chunk_embedding(pool, *cid, resp.embeddings[i].clone())",
    "                    for (i, (cid, _)) in batch.iter().enumerate() {\n                        repo::set_chunk_embedding(pool, lib, *cid, resp.embeddings[i].clone())",
))

subs.append((
    "                    tracing::warn!(error = %e, \"嵌入批次失败，标记 embed_failed\");\n                    for (cid, _) in batch {\n                        repo::set_chunk_failed(pool, *cid)",
    "                    tracing::warn!(error = %e, \"嵌入批次失败，标记 embed_failed\");\n                    for (cid, _) in batch {\n                        repo::set_chunk_failed(pool, lib, *cid)",
))

subs.append((
    "    repo::set_ready_document(pool, doc_id)",
    "    repo::set_ready_document(pool, lib, doc_id)",
))

subs.append((
    "        let wiki = crate::wiki::WikiService::new(pool.clone(), registry.clone());\n        let _ = wiki.ingest_document(doc_id).await;",
    "        let wiki = crate::wiki::WikiService::new(pool.clone(), registry.clone());\n        let _ = wiki.ingest_document(lib, doc_id).await;",
))

for i, (old, new) in enumerate(subs, 1):
    assert old in s, 'MISS %d: %s' % (i, old[:70])
    s = s.replace(old, new, 1)

# mark_failed 从 async fn 改为闭包后，调用点适配（async fn 返回 Future 需要 .await；闭包也是 async 块…）
# 这里闭包体用 .await 会报错（非 async 上下文）——改用同步即可（repo 调用本身是 async）
# 因此调用点保持 .await 形态：改为 async move 闭包不行（借用），直接改为 async fn 带 lib 参数最稳。
# 还原为 async fn（带 lib）：
old = "    let mark_failed = |ctx: &JobContext, msg: String| {\n        let _ = repo::mark_failed_document(ctx.pool(), lib, doc_id, &msg).await;"
assert old in s
s = s.replace(old, "    async fn mark_failed(ctx: &JobContext, lib: Uuid, doc_id: Uuid, msg: &str) {\n        let _ = repo::mark_failed_document(ctx.pool(), lib, doc_id, msg).await;", 1)

# 调用点：3 处改带 lib
s = s.replace("mark_failed(&ctx, m).await;", "mark_failed(&ctx, lib, doc_id, &m).await;")
s = s.replace("mark_failed(&ctx, m.clone()).await;", "mark_failed(&ctx, lib, doc_id, &m).await;")

open(p, 'w', encoding='utf-8', newline='').write(s)
print('pipeline patched:', len(subs), 'core + callsites')
