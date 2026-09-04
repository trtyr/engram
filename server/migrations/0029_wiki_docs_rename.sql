-- 0029: knowledge 并入 wiki——documents/chunks 表改名
ALTER TABLE documents RENAME TO wiki_documents;
ALTER TABLE chunks RENAME TO wiki_chunks;
ALTER INDEX idx_documents_status RENAME TO idx_wiki_documents_status;
ALTER INDEX idx_documents_created RENAME TO idx_wiki_documents_created;
ALTER INDEX idx_chunks_tsv RENAME TO idx_wiki_chunks_tsv;
ALTER INDEX idx_chunks_embedding RENAME TO idx_wiki_chunks_embedding;
