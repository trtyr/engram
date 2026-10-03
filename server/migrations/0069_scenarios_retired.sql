-- 0069: P012 agentic 场景软删——scenarios 加 retired_at 列。
--
-- 背景：organize agentic 化后模型获得场景删除能力，但「删除一律软删」是结构
-- 不变量（用户拍板 2026-10-03）——模型无物理删权限；物理解散仅保留 converge
-- 确定性路径（活跃成员=0）。retired_at 非空 = 已退役（读侧全部过滤）。

ALTER TABLE scenarios ADD COLUMN IF NOT EXISTS retired_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_scenarios_active ON scenarios (updated_at DESC)
    WHERE retired_at IS NULL;
