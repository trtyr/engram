# 决策 001 · L0→L1 前置保险采用 JEV 决策模型（2026-10-03）

## 决定
用户记忆的 L0→L1 蒸馏链路前置保险改用 **OpenRouter JEV 决策模型**（`typesafe/jev-1.13`，
System One：choice/noul/score 三原语，typed 输出+概率，输出 token 免费）。

1. **extract claim 后、chat 精抽前**插入 JEV noul 保险判定，按概率三档路由：
   `<0.3` 拒绝跳过（事件流记「JEV 拒绝 p=xx」）/ `0.3~0.5` 照常精抽但产物提级
   needs_review（复用现有待审通道）/ `≥0.5` 放行。
2. **归因回执（原 T011）由 JEV choice 五分类一次请求承接**（user_facts/project_internal/
   transient/learning/chitchat），保险判定与拒绝理由同请求产出。
3. **配置面**：Web 设置「AI 功能」新增 JEV 区块——接口**仅支持 OpenRouter**（不提供
   provider 选择器，默认且唯一 OpenRouter），用户填写 API key（加密存储），加 enabled
   开关与阈值参数。
4. **阶段二（L0→L1 落地后）**：arbitrate 三判 choice 化、consolidate 近重复 noul 化。

## 依据（两轮评测，2026-10-03，107 次调用 <$0.002）
- 构造用例 10/10（noul 分离度 KEEP≥0.58 / REJECT≤0.24；choice conf 0.84-1.0）
- 生产回放 25 段一致率 84%（KEEP 18/20、REJECT 3/5）；方差 spread≤0.06，阈值路由可行；
  2 条疑似历史 extract 误杀待复核
- arbitrate 三判 7/7（真实 superseded 对 contradicts conf 0.92；独立对 new 全对）

## 边界
- extract 开放抽取、persona/场景生成**不换**——保持 chat 模型（JEV 只做封闭判断）
- 中文效果以本次两轮评测为准；后续 prompt v9+ 调整时用 T011 回执数据持续校准
- 凭据：`openrouter/engram`（credentials 域）；生产配置走设置面，不由本决策携带值
