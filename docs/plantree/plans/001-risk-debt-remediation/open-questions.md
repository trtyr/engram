# Open Questions（等你拍板）

> 只留未决。已决问题见 decisions/（001 敏感边界+回填 · 002 规划树不入库，均 2026-09-28 拍板）。

## Q004 · T005 存量回填目标实例（生产 vs 本机）

- **背景**：T004 敏感统一放开已落地并提交（bde6092），历史被排除的敏感原子需跑 `POST /memory/distill {mode:"rebuild"}`（full_rebuild persona + organize）补进场景/画像
- **选项**：A 生产 engram.trtyr.top（数据真实，但动生产 + LLM 账单已知情）· B 本机 ~/.engram（安全，但本机数据不全、回填代表性差）
- **状态**：未决（2026-10-01 自 roadmap T005「待确认」补录——roadmap 管任务状态，待拍板归此）
