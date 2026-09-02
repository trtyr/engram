# 未决问题

## Q1 会话级 sensitive 的粒度

- 整会话 sensitive，还是单轮 turn sensitive？
- 若单轮：蒸馏产物如何映射回"哪一轮"？还是所有产物统一继承会话级标记？

## Q2 文件导入的解析规则

- 纯文本文件怎么切成 turns？按空行？按角色前缀（"我：""谢乐："）？
- JSONL 的 role 字段枚举（user/assistant/system）是否复用 session-write 的 turn 语义？

## Q3 权限收窄后 sensitive 锁的时机

- 若"会话级敏感标记"没先落地，收回 `atom-patch --sensitive` 会留下泄露窗口
- 顺序建议：先做会话级敏感标记，再收 sensitive 的 atom-patch？

## Q4 41 条无溯源原子的处置

- 补 source_refs（伪造溯源）？还是打标"历史直写，溯源未知"？
- 对画像/场景的既有影响是否可接受？

## Q5 关系 source 混存去重

- 38 manual + 22 distill 的语义重复，consolidate 是否自动统一？
