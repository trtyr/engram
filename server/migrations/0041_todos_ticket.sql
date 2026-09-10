-- 0041：todos 双形态——kind 区分 todo（微软式行动项）/ ticket（工单）。
-- todo 保持轻量两态（open/done/archived）；ticket 走结构化问题跟踪：
--   字段：severity(P0-P3) / symptom(症状) / reproduce(复现) / acceptance(验收) / resolution(解决记录)
--   状态机：open → confirmed → in_progress → resolved → verified（+archived）
-- 联合 CHECK 按 kind 分支强制状态机（数据库层拒绝非法组合，不只靠应用层）。
-- 存量行零影响：kind 默认 'todo'，工单字段对 todo 型无感。

ALTER TABLE todos ADD COLUMN kind text NOT NULL DEFAULT 'todo';
ALTER TABLE todos ADD COLUMN severity text;
ALTER TABLE todos ADD COLUMN symptom text NOT NULL DEFAULT '';
ALTER TABLE todos ADD COLUMN reproduce text NOT NULL DEFAULT '';
ALTER TABLE todos ADD COLUMN acceptance text NOT NULL DEFAULT '';
ALTER TABLE todos ADD COLUMN resolution text NOT NULL DEFAULT '';
ALTER TABLE todos ADD COLUMN resolved_at timestamptz;

-- 清理旧 status CHECK（0035 内联自动命名；多库形态差异——动态删除，0039 教训）
DO $$ DECLARE c record;
BEGIN
  FOR c IN SELECT conname FROM pg_constraint
           WHERE conrelid = 'todos'::regclass AND contype = 'c' AND conname LIKE '%status%'
  LOOP
    EXECUTE format('ALTER TABLE todos DROP CONSTRAINT %I', c.conname);
  END LOOP;
END $$;

-- 新状态机（联合 CHECK）+ kind 合法性 + severity 枚举 + 工单解决必填 resolution
ALTER TABLE todos ADD CONSTRAINT todos_kind_check
  CHECK (kind IN ('todo', 'ticket'));
ALTER TABLE todos ADD CONSTRAINT todos_status_check CHECK (
  (kind = 'todo' AND status IN ('open', 'done', 'archived'))
  OR
  (kind = 'ticket' AND status IN ('open', 'confirmed', 'in_progress', 'resolved', 'verified', 'archived'))
);
ALTER TABLE todos ADD CONSTRAINT todos_severity_check
  CHECK (severity IS NULL OR severity IN ('P0', 'P1', 'P2', 'P3'));
ALTER TABLE todos ADD CONSTRAINT todos_ticket_resolution_check
  CHECK (kind <> 'ticket' OR status NOT IN ('resolved', 'verified') OR btrim(resolution) <> '');

CREATE INDEX idx_todos_kind ON todos (kind);
