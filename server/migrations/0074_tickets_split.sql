-- 0074: 工单拆独立表——项目绑定制（2026-10-07 拍板）。
-- 工单与待办的本质区别：待办随便写无前置；工单必须绑定已有项目，绑定不了就不进工单。
-- 1) tickets 独立表：project_id NOT NULL FK（项目删 → 工单级联删，绑定没了即悬空），
--    severity/resolution/六态状态机随迁（0041 的联合 CHECK 拆为 tickets 自有 CHECK）。
-- 2) 存量工单（todos 里 kind='ticket'）直接删——项目绑定制下无绑定即悬空，不留旧兼容。
-- 3) todos 瘦身回 0035 定位（轻量行动项）：drop kind/ticket 时代字段 + project_hint。
-- 4) short_no 序列：tickets 自有序列，起点 = 全体 todos 现有最大短号（EN- 编号跨表连续）。
-- 5) ticket_events 外键重挂 → tickets(id)（存量行已随工单级联清空）。

-- ---------- tickets ----------
CREATE TABLE tickets (
    id          uuid PRIMARY KEY,
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    short_no    integer NOT NULL,
    title       text NOT NULL,
    body        text NOT NULL DEFAULT '',
    status      text NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','confirmed','in_progress','resolved','verified','archived')),
    severity    text,
    symptom     text NOT NULL DEFAULT '',
    reproduce   text NOT NULL DEFAULT '',
    acceptance  text NOT NULL DEFAULT '',
    resolution  text NOT NULL DEFAULT '',
    resolved_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT tickets_severity_check
        CHECK (severity IS NULL OR severity IN ('P0','P1','P2','P3')),
    -- 解决必填（0041 语义随迁）：进 resolved/verified 必须留解决记录
    CONSTRAINT tickets_resolution_check
        CHECK (status NOT IN ('resolved','verified') OR btrim(resolution) <> '')
);
CREATE INDEX idx_tickets_project ON tickets (project_id);
CREATE INDEX idx_tickets_status ON tickets (status);

-- 短号序列：EN- 编号全局连续——起点取 todos 现有最大短号（工单从此往后编）
CREATE SEQUENCE seq_tickets_short_no OWNED BY tickets.short_no;
SELECT setval('seq_tickets_short_no',
    GREATEST((SELECT COALESCE(MAX(short_no), 0) FROM todos), 1),
    true);
ALTER TABLE tickets ALTER COLUMN short_no SET DEFAULT nextval('seq_tickets_short_no');
CREATE UNIQUE INDEX idx_tickets_short_no ON tickets (short_no);

-- ---------- 存量退役：无绑定即悬空，直接删 ----------
-- 删除顺序安全：todo_links / ticket_events 都 ON DELETE CASCADE，随行清空
DELETE FROM todos WHERE kind = 'ticket';

-- ---------- ticket_events 外键重挂 ----------
ALTER TABLE ticket_events DROP CONSTRAINT IF EXISTS ticket_events_ticket_id_fkey;
ALTER TABLE ticket_events
    ADD CONSTRAINT ticket_events_ticket_id_fkey
    FOREIGN KEY (ticket_id) REFERENCES tickets(id) ON DELETE CASCADE;

-- ---------- todos 瘦身：回 0035 轻量行动项定位 ----------
DROP INDEX IF EXISTS idx_todos_kind;
ALTER TABLE todos DROP CONSTRAINT IF EXISTS todos_kind_check;
ALTER TABLE todos DROP CONSTRAINT IF EXISTS todos_status_check;
ALTER TABLE todos DROP CONSTRAINT IF EXISTS todos_severity_check;
ALTER TABLE todos DROP CONSTRAINT IF EXISTS todos_ticket_resolution_check;

ALTER TABLE todos DROP COLUMN IF EXISTS kind;
ALTER TABLE todos DROP COLUMN IF EXISTS severity;
ALTER TABLE todos DROP COLUMN IF EXISTS symptom;
ALTER TABLE todos DROP COLUMN IF EXISTS reproduce;
ALTER TABLE todos DROP COLUMN IF EXISTS acceptance;
ALTER TABLE todos DROP COLUMN IF EXISTS resolution;
ALTER TABLE todos DROP COLUMN IF EXISTS resolved_at;
ALTER TABLE todos DROP COLUMN IF EXISTS project_hint;

ALTER TABLE todos ADD CONSTRAINT todos_status_check
    CHECK (status IN ('open', 'done', 'archived'));
