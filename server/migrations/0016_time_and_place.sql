-- 0016: 时间表达力 + place 实体类型（测试方第二波设计报告 议题二/六）
-- 1) 原子的事件时间与有效期：created_at 是记录时间不是事件时间——"下周三骑行"
--    这类相对时间由 extract 阶段以当天为锚解析成绝对时间挂 occurred_at；
--    valid_until 到期后检索可降权/过滤（本迁移只立列，打分策略下轮）。
-- 2) 实体 kind 增 place：地点是用户世界的高频透镜（淀山湖案），此前塞不进
--    person/project/topic/group 只能丢弃。

ALTER TABLE atoms ADD COLUMN occurred_at timestamptz NULL;
ALTER TABLE atoms ADD COLUMN valid_until  timestamptz NULL;

ALTER TABLE entities DROP CONSTRAINT entities_kind_check;
ALTER TABLE entities ADD CONSTRAINT entities_kind_check
    CHECK (kind IN ('person', 'project', 'topic', 'group', 'place'));
