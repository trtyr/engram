-- 0030: knowledge 残留清理——表约束名归 wiki 命名 + api_keys 默认 scopes 去 knowledge
-- 0029 改名表后，约束名仍沿用 documents_/chunks_ 前缀（ALTER TABLE RENAME TO 不改约束名）；
-- api_keys.scopes 默认值仍含已删除的 knowledge scope。二者均为命名残留，功能无影响。

ALTER TABLE wiki_documents RENAME CONSTRAINT documents_pkey TO wiki_documents_pkey;
ALTER TABLE wiki_documents RENAME CONSTRAINT documents_sha256_key TO wiki_documents_sha256_key;
ALTER TABLE wiki_documents RENAME CONSTRAINT documents_status_check TO wiki_documents_status_check;
ALTER TABLE wiki_chunks RENAME CONSTRAINT chunks_pkey TO wiki_chunks_pkey;
ALTER TABLE wiki_chunks RENAME CONSTRAINT chunks_document_id_fkey TO wiki_chunks_document_id_fkey;
ALTER TABLE wiki_chunks RENAME CONSTRAINT chunks_document_id_seq_key TO wiki_chunks_document_id_seq_key;

ALTER TABLE api_keys ALTER COLUMN scopes SET DEFAULT '["memory","wiki","codegraph"]';
