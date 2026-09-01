# Open Questions — circle

## 1. 实体关系升级：建不建类型化关系表？（✅ 已拍板：选 A）

用户 2026-09-01 拍板 **A 完整类型化关系表**（推翻 e2174fda9b28 的「关系语义写摘要」MVP 取舍）。已落地：迁移 0021 + 蒸馏抽取 + 关系 CRUD。

## 2. 时间轴视图的形态（✅ 已拍板：全局记忆时间轴）

用户 2026-09-01 拍板「全局记忆时间轴」独立视图（GET /memory/timeline + 图谱/时间轴切换），非实体时间线。

## 3. 批量操作的破坏性边界（✅ 已拍板：erase scope + confirm 短语）

已落地：POST /memory/entities/batch 破坏性批量删除需 erase scope + confirm="批量删除"（与 deep purge 同构，非两阶段 arm/token——批量半径小，二次确认足够）。

## 4. 数据密度瓶颈的应对

圈子图的价值依赖数据密度（当前 16 实体全 atom_count=1）。是「等数据自然积累」，还是「降级为列表为主、图为辅」过渡？见 topics/data-density.md。
