-- 0034: 实体归档标记（D16）——会话 void 级联到实体层。
-- 语义：实体失去全部活跃原子（被遗忘）时打 archived_at，从列表/检索/图谱隐身；
-- 蒸馏重新挂链同名实体时清标记复活。实体本身可审计（GET 单查仍可见）。

ALTER TABLE entities ADD COLUMN archived_at timestamptz;
