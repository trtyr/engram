-- 会话级敏感标记：写会话时声明 sensitive，蒸馏产物自动继承
-- （消除「AI 写敏感内容 → 等 30s 防抖 + extract 跑完 → 再手动锁」之间的裸奔窗口）
ALTER TABLE raw_sessions ADD COLUMN sensitive boolean NOT NULL DEFAULT false;
