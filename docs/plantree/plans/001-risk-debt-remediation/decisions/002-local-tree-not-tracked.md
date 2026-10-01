# Decision 002 · 规划树与 AGENTS.md 不入库

- **日期**：2026-09-28
- **拍板人**：trtyr（对 Q003「规划树不用入库」）

## Decision

`docs/plantree/` 与仓库根 `AGENTS.md` 保持 untracked，**不进 git**——仓库维持「只留 README 门面」的既定方针（README:143-150）；本地规划树是单机私密工作面。

## Consequences

- ✅ 仓库门面干净，符合「文档即数据、权威在 engram」方针
- ⚠️ AGENTS.md 的档案索引段只在本机生效（其他机器 clone 仓库看不到）——跨机器规划暂不可用，接受
- ⚠️ 本地树无版本控制——重要拍板以 decisions/ 为准，必要时同步一份定稿进 engram 档案（决策类 doc）

## Alternatives

- 入库多机同步：被否（Q003 拍板）
