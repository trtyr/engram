-- 0058: 资产域（assets）+ 项目位置的真引用（asset_id）。
--
-- 背景（2026-09-21 用户拍板，见《项目与资产模型 · README》§2）：
-- 资产（主机 / 云实例 / 域名 / U盘 / 账号）是「我拥有的、可以被操作的东西」——它没有「收尾」、
-- 身份唯一、会被多个项目引用、要留变更痕。这与 projects 的治理规则不同（有目标、能收尾、级联删），
-- 所以独立成第 9 个域；项目侧只**引用**资产、不复制身份（唯一事实源铁律）。
--
-- 本迁移做三件事：
--   一、建 assets 表（kind 值域 CHECK 与 core 常量 ASSET_KINDS 一一对应）；
--   二、project_locations 加 asset_id 引用列；
--   三、存量清洗：把「主机清单」project 的 5 条台账（4 主机 + U盘）灌成资产条目，并把 3 行漂移的
--       location（`trtyr-mac` / `demotestdeMacBook-Air.local`）按别名归一到同一资产。
--       条件式执行——只在源项目存在时灌（干净库 / 其他实例 no-op）；幂等可反复跑（与 0057 同口径）。

CREATE TABLE IF NOT EXISTS assets (
    id         uuid PRIMARY KEY,
    -- 值域事实源 = core 的 `ASSET_KINDS` 常量（改值域 = 改常量 + 一条迁移）
    kind       text NOT NULL
               CHECK (kind IN ('host','cloud','domain','account','device','other')),
    name       text NOT NULL,                 -- 台账名（唯一）
    aliases    text[] NOT NULL DEFAULT '{}',  -- 别名：主机名 / ssh 别名 / 历史写法（引用匹配与历史归一的依据）
    ip         text NOT NULL DEFAULT '',      -- 规范地址（公网或组网 IP；会变，变化走 update）
    os         text NOT NULL DEFAULT '',
    note       text NOT NULL DEFAULT '',
    fields     jsonb NOT NULL DEFAULT '{}',   -- 轻结构扩展（序列号 / 规格 / 厂商…按需）
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_assets_name ON assets (name);
CREATE INDEX IF NOT EXISTS idx_assets_kind ON assets (kind);

-- 引用列：RESTRICT（不是 CASCADE）——资产是唯一事实源；删一个还被项目用着的资产必须**显式解绑**，
-- 不能静默把项目的引用位清空（与「删项目级联删位置」方向相反，这是有意的）。
ALTER TABLE project_locations
    ADD COLUMN IF NOT EXISTS asset_id uuid REFERENCES assets(id) ON DELETE RESTRICT;
CREATE INDEX IF NOT EXISTS idx_project_locations_asset ON project_locations (asset_id);

-- 存量清洗：主机清单 5 条台账 → 资产条目（只在源项目存在时执行；ID 用 gen_random_uuid，
-- 重跑靠 ON CONFLICT (name) 幂等）。名称取干净台账名，历史写法（含文档标题）进别名。
INSERT INTO assets (id, kind, name, aliases, ip, os, note)
SELECT v.id, v.kind, v.name, v.aliases, v.ip, v.os, v.note
  FROM (VALUES
      (gen_random_uuid(), 'host', 'MacBook Air M1',
       ARRAY['trtyr-mac','demotestdeMacBook-Air.local','demotestdeMacBook-Air','demotest的MacBook Air','01 · MacBook Air M1（本机）'],
       '100.74.134.42', 'macOS 26.3', 'M1 8C / 8GB / 228GiB；本机（随用户移动）；LAN 会变'),
      (gen_random_uuid(), 'host', 'Legion 5',
       ARRAY['DESKTOP-3M7DKO9','Legion','02 · Legion 5 · Windows'],
       '100.80.65.64', 'Windows 10 22H2', 'Ryzen 7 5800H / 16GB / 1TB+512GB NVMe；上海出租屋'),
      (gen_random_uuid(), 'cloud', '腾讯云 · 北京',
       ARRAY['tencent-beijing','VM-0-14-opencloudos','03 · 腾讯云 · 北京'],
       '82.157.147.224', 'OpenCloudOS 9.4', '腾讯云 CVM 北京 4C / 3GB / 60GB；ssh 22'),
      (gen_random_uuid(), 'cloud', '腾讯云 · 新加坡',
       ARRAY['tencent-sg','VM-8-5-opencloudos','04 · 腾讯云 · 新加坡'],
       '43.163.80.102', 'OpenCloudOS 9.6', '腾讯云 CVM 新加坡 2C / 3GB / 60GB；ssh 2222'),
      (gen_random_uuid(), 'device', 'U盘 trtyr',
       ARRAY['trtyr U盘','05 · U盘 trtyr（256G · exFAT）'],
       '', '', '256G exFAT；代码工作区（外置盘）')
  ) AS v (id, kind, name, aliases, ip, os, note)
 WHERE EXISTS (SELECT 1 FROM projects WHERE name = '主机清单')
ON CONFLICT (name) DO NOTHING;

-- 位置登记的自由文本 host 归一到资产条目（按 名称 或 别名 匹配）——漂移引用在此收敛。
UPDATE project_locations l
   SET asset_id = a.id,
       updated_at = now()
  FROM assets a
 WHERE l.asset_id IS NULL
   AND (l.host = a.name OR l.host = ANY (a.aliases));

-- ─────────────────────────────────────────────────────────────────────────────
-- 二部：项目关联（project_links）——工作线之间的**一层**有向关联。
--
-- 背景（2026-09-21 用户拍板，见《项目与资产模型 · README》§2.3）：标签表达「同类」（无向、无约束），
-- 表达能力不了**归属**（A 属于 B ⇏ B 属于 A）；所以要归属就得落**类型化有向边**。
--
-- kind 值域（事实源 = core 的 PROJECT_LINK_KINDS 常量）：
--   part_of  = from 隶属 to（「数据集团-驭元 POC」part_of「上海数据集团」）
--   related  = 相关（语义无向；存一行，查询时两向合并去重）
--
-- 纪律（写进表约束，别靠自觉）：
--   · 自环禁止（from = to 无意义）——CHECK 拦；
--   · 同向同类重复禁止——唯一索引拦；
--   · 不做无限深树：一层展示够用，深层靠项目内文档树承载（README §3 第 4 条）。
-- 删任一端项目 → 关联级联清（CASCADE；关联本身不是资产，没有留痕要求）。
--
-- 与资产同属一次模型变更（工作线 ↔ 资产），故合并在同一支迁移里（2026-09-22 收口）。
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS project_links (
    id           uuid PRIMARY KEY,
    from_project uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    to_project   uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind         text NOT NULL CHECK (kind IN ('part_of','related')),
    note         text NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT project_links_no_self CHECK (from_project <> to_project)
);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_project_links
    ON project_links (from_project, to_project, kind);
CREATE INDEX IF NOT EXISTS idx_project_links_from ON project_links (from_project);
CREATE INDEX IF NOT EXISTS idx_project_links_to ON project_links (to_project);
