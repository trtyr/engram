-- 0035: 待办域（第七域）——不绑定项目的临时任务/灵感速记。
-- 定位：轻量「速记→做完勾掉」，非工单（无指派/SLA/流程）。
-- 场景：灵感记录 / 学习计划 / 系统操作 / 问题排查——靠 tags + priority + due_at 表达。

CREATE TABLE todos (
    id           uuid PRIMARY KEY,
    title        text NOT NULL,                 -- 一句话标题（必填）
    body         text NOT NULL DEFAULT '',      -- 详情（可选 markdown）
    status       text NOT NULL DEFAULT 'open'
                 CHECK (status IN ('open','done','archived')),
    priority     text NOT NULL DEFAULT 'normal'
                 CHECK (priority IN ('low','normal','high')),
    tags         text[] NOT NULL DEFAULT '{}',
    due_at       timestamptz,
    project_hint text,                          -- 可选：相关项目名提示（纯文本，不做 FK 绑定）
    done_at      timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_todos_status ON todos (status);
CREATE INDEX idx_todos_tags ON todos USING gin (tags);
CREATE INDEX idx_todos_due ON todos (due_at) WHERE status = 'open';
