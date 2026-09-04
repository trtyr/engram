-- 0027: 位置元数据补齐——ip + os。
-- 位置是项目元数据（「项目在哪」的身份信息），不是项目内容。
-- ip 兼容内网/公网/IPv6，只校验非空不校验格式；os = 操作系统（macOS/Ubuntu/Windows…）。
-- 旧行 ip/os 落空串，前端展示为空、由用户编辑补上。
ALTER TABLE project_locations ADD COLUMN ip text NOT NULL DEFAULT '';
ALTER TABLE project_locations ADD COLUMN os text NOT NULL DEFAULT '';
