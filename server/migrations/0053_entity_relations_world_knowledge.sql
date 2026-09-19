-- 0053 常识边层级模型（2026-09-18 收录哲学线 task-8）：
-- entity_relations.source 扩 world_knowledge——relation_backfill 从世界知识补的实体间关系
--（如 权志龙 --member_of--> BIGBANG：两个实体均被用户会话发现，但关系本身来自 LLM 常识
-- 而非用户记忆原文）。常识边只作圈子图渲染的语境补充（虚线淡显 + 第 2 层剪枝），
-- 永不升级为记忆原子、不当画像证据（收录哲学「二度世界」层级模型）。
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'entity_relations_source_check'
           AND conrelid = 'entity_relations'::regclass
           AND pg_get_constraintdef(oid) LIKE '%world_knowledge%'
    ) THEN
        ALTER TABLE entity_relations DROP CONSTRAINT IF EXISTS entity_relations_source_check;
        ALTER TABLE entity_relations ADD CONSTRAINT entity_relations_source_check
            CHECK (source IN ('distill', 'manual', 'world_knowledge'));
    END IF;
END $$;
