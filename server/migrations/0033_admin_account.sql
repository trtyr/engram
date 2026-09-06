-- 0033: 管理员账号（单用户）——完整的用户名 + 密码登录。
-- 单行表（id=1）。首次由 env 密码播种（username='admin'）或登录页初始化表单创建；
-- 之后以本表为准（env 失效），账号/密码经管理页修改。

CREATE TABLE admin_account (
    id            int PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    username      text NOT NULL,
    password_hash text NOT NULL,             -- pbkdf2-sha256$iter$salt$hash
    updated_at    timestamptz NOT NULL DEFAULT now()
);
