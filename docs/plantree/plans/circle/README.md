# Plan — circle（圈子能力审计与强化）

## Scope

圈子（/circle）——记忆模型「一坐标系」维度的独立页，按 WHO/WHAT（实体：person/project/topic/group/place）浏览记忆，与「一架梯子」（/memory L0→L3）正交。

**In**：圈子的四维审计（功能/前端/缺口/API）、P1/P2/P3 强化清单、实施轮（审计过目后另开）、实体关系/检索/时间轴等全新能力候选。

**Out**：/memory 梯子页本身、知识库/Wiki/代码图谱、蒸馏算法（已有）、编辑分权（已有）、harness 侧自动节律钩子（**非本系统能力**，见 memory-rhythm）。

## Authority

- 审计报告（本轮产出）：[docs/design/circle-audit.md](../../../design/circle-audit.md)
- 实施轮：用户过目 P1/P2/P3 清单后另开 goal。

## File Map

- `roadmap.md` — 审计完成 + P1/P2/P3 分级清单（实施轮的待办源）
- `open-questions.md` — 需用户拍板的大项
- `topics/relation-model.md` — 实体关系升级（共现→语义）候选方案胶囊
- `topics/data-density.md` — 数据密度瓶颈（图的灵魂，先读）

## 基线（2026-09-01 审计）

- 活体：生产栈 :19180，16 实体（全 atom_count=1）/ 13 共现边（全 weight=1）/ 19 迁移 / 69 路径。
- 前端：Circle.tsx(32) + Galaxy.tsx(520) + EntityGalaxy.tsx(147，lazy sigma 157kB)。
- API：entities 面 5 路径 / 9 方法注册（list/create/graph/get/patch/delete/atoms/merge/forget），全部 require_memory。
