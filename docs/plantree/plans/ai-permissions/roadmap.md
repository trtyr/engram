# Roadmap

## Done（已实测验证）

- **session-write → 蒸馏自动加工**（2026-09-02 实测）：3 轮用户口述 → 抽出 10 条原子 + 22 条关系（source=distill）+ 画像更新；判重正确（7 条重复归档）
- **correction 走会话**（2026-09-02 实测）：写"纠正对话" → 蒸馏自动归档旧原子 + 生成新原子 + superseded_by 取代链 + 画像 identity v4 更新
- **敏感保护三链**（P3，早前验证）：search/context 默认隐身，--reveal 可见；2 条 sensitive 原子（鹿鞭地黄丸 + 家庭地址）正确锁定，0 遗漏
- **AI 权限收窄**（2026-09-02）：收回 atom-add / entity-add / entity-rel-add / entity-attach 直写，只留会话写入 + 保护类（sensitive/status）+ distill 触发；删实体/摘原子/删关系收进 erase scope（fa88777）
- **会话级敏感标记**（73d0d65）：写会话声明 sensitive，蒸馏产物自动继承（消除"异步蒸馏完再手动锁"的泄露窗口）
- **文件批量导入为会话**（1f082e7，二期②）：POST /memory/sessions/import，JSONL/文本 → turns → source=import；蒸馏感知 import 过滤对方观点（测试方实测：谢乐负责售前方案被正确过滤）
- **过期自动降权/过滤**（d9316b1，二期①）：search_atoms 对 valid_until<now() 的原子 ×0.5 排后；context_pack 双路径硬过滤过期；归档先不做（用户选 A 自动淡忘不标注）
- **检索时间范围过滤**（9c45889，二期③）：SearchRequest from/to，COALESCE(occurred_at, created_at) 时间窗；session GET 暴露 metadata.source

## Next

（清空——权限收窄 + 二期三项全部落地验收，系统进入稳定运行）

## Deferred

- 清理 41 条无溯源原子（直写残留，数据迁移需评估影响）
- FDE 误译修正（画像把 Forward Deployed Engineer 译成"前端部署工程师"）
- 关系 source 混存（38 manual + 22 distill）去重
- 小王→小程序误识别（人名实体抽取有误，低优先级，测试方 2026-09-02 顺手记）
- 同名不同 kind 实体自动合并（correction 修实体 kind 留重复实体：去重只按 name+kind，不同 kind 就新建——鱼韵 person→group 已手工 merge 清一次，根因待下批修）
- 微信导入（用户尚未想清楚加解密方案，搁置，等用户想清楚再催）
- 多 agent 画像（当前单 agent 场景用不上，往后放）
