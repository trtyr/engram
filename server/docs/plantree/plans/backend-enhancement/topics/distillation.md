# Topic: 蒸馏与质量

覆盖 R7/R14/R17。核心问题：蒸馏是「让经验复利」的引擎，现在有长会话覆盖率和人类修正存活两个真实缺口。

## 现状盘点

- L0→L3 链：`extract_atoms → arbitrate_atoms → organize_scenarios → distill_persona` + `consolidate`，提示词版本化（`PromptId(name, version)`）。
- 防抖：30s 窗口（`trigger_auto_extract`），幂等键 `extract-debounce-{bucket}`。
- 人工保护：wiki 有 `origin=human`；memory 有 `update_atom` 状态机（active/archived）、`erase_session` 来源失效标记。

## R7 extract 长会话覆盖率

- 现状：`extract.rs` 把认领的**所有会话全拼接**进一个 user 提示（`numbered` 变量），会话多/长时超上下文，中间段静默丢失。
- 依据：评论区 XBlueSky 教训——「长会话蒸馏需要覆盖率，而不是一个巨大的总结 prompt」。
- 目标：按会话/轮次切有界跨度，每段要么抽取要么显式标记「无持久洞察」，不让上下文耗尽静默丢段。
- 实现方向：extract 前按字符预算切 span，多段并行或串行 LLM 调用，每段产出归并。

## R14 pin 存活增强

- 现状：wiki 的 `origin=human` 是整页级保护（LLM 不覆盖人写的页，只发提案）；memory 的 `update_atom` 允许人改 active 原子，但下次 distill 可能被 LLM 重写。
- 依据：评论区 huachen-wang 教训 4——存「修正意图（claim）+ 锚定小节 + 重编译后核对」，而非 diff。
- 目标：记录人类修正的 claim（纠正了什么、锚定哪里），重编译时核对：仍满足→保留；被反驳→浮出给人；小节消失→标孤儿。
- 注意：单用户场景优先级 P2，但方向正确，是「人类修正存活」的正解。

## R17 Wiki↔Memory 互操作

- 现状：memory（偏好/事实/画像）与 wiki（实体/概念/综合）两域完全隔离，无桥。
- 目标：
  - wiki 页面蒸馏出「事实原子」回灌 memory（wiki 读到的稳定知识 → 长期记忆）。
  - memory 的 L1/L3 可触发 wiki 建页（画像里的稳定结论 → wiki 论点页）。
- 价值：打通「对话记忆」和「文档知识」两个复利循环，这是「更强大」的质变点。
