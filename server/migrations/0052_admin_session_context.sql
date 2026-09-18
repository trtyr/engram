-- 0052: 管理员会话上下文（活跃会话列表展示浏览器/IP——账号与安全页增强 2026-09-18）
ALTER TABLE admin_sessions ADD COLUMN ip text;
ALTER TABLE admin_sessions ADD COLUMN user_agent text;
