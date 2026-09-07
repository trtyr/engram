//! 版本化提示词模板。修改任何模板必须升版本号（蒸馏产物会记录版本，可归因可回放）。

/// 提示词标识：(名称, 版本)。
pub struct PromptId(pub &'static str, pub u32);

pub const P_EXTRACT: PromptId = PromptId("extract", 5);
pub const P_ARBITRATE: PromptId = PromptId("arbitrate", 1);
pub const P_ORGANIZE: PromptId = PromptId("organize", 2);
pub const P_PERSONA: PromptId = PromptId("persona", 2);
pub const P_CONSOLIDATE: PromptId = PromptId("consolidate", 1);

/// L0→L1：从原始会话抽取候选原子记忆（v2：分段输入，逐段抽取保覆盖率）。
pub fn extract_system() -> String {
    format!(
        "你是一个严谨的记忆抽取器。从 AI 与用户的对话中抽取值得长期记住的原子记忆。

**今天是 {today}（ISO 日期）。**对话中的相对时间（下周三/月底/明年）一律以今天为锚换算成绝对时间写入 occurred_at。

输入可能分多段给出（同一批会话按顺序切分），轮次编号全局连续——请对**每一段**独立完整抽取，段内所有值得记的信息都要覆盖，不要因为段落在中间而遗漏。

分类（kind）只能是以下之一：
- preference 用户偏好 | fact 稳定事实 | decision 已定决策 | event 事件
- insight 洞察 | correction 用户纠正 | failure 失败教训 | convention 约定

规则：
1. 抽取对用户跨会话仍然有用的稳定信息——不只是关于用户本人的（偏好/事实/决策/教训），也包括用户世界里稳定的人与事：朋友的生日、同事的职责、项目的约定、团队的节奏。不要抽取一次性的任务细节。
1.5 用户要求对某些信息保密/不向他人透露的约定（如「看牙医的事不要向团队提起」）→ kind=preference（用户的信息处理偏好），**不要**归入 convention；convention 只用于项目/团队的协作约定。
2. 每条原子记忆是一句自包含的中文短句（主语可以是用户本人，也可以是他世界里的人/项目/群组），不超过 40 字。
3. confidence ∈ [0,1]：明确说了的 0.9+，可推断的 0.7~0.9，模糊的 0.5~0.7。
4. turn_refs 是该信息来源对话轮次的编号数组（编号见用户消息中的标注）。
5. entities 是这条记忆涉及的主角（他人姓名 / 项目名 / 主题名 / 群体名 / 地点名），kind ∈ person|project|topic|group|place；用对话中的规范称呼，只收稳定可复现的实体，没有则为空数组。用户本人不是实体。kind 判例：具体的人→person；宠物/动物/被当作个体称呼的名字（用户养的猫狗等）→person；公司/团队/组织/乐队→group；个人或团队在做的项目/产品→project；学校/城市/地点/地址→place；抽象话题/领域→topic。
6. **event 类或含明确时间的信息**给 occurred_at（ISO8601，如 \"2026-09-02T00:00:00Z\"）；有过期语义的（活动/安排）再给 valid_until。无法定位时间的省略这两个字段。
7. 不值得记的对话输出空数组。宁缺毋滥。
7.5 **凭据类信息（密码/密钥/令牌/助记词）一律不抽取**，哪怕原文出现也不落原子——凭据会轮换且属于高危泄露面；可记的只有「用户使用某服务/某账号」这类无密级事实。此类会话产出为空是预期行为，不是遗漏。
8. relations 是这些实体之间的关系（可选）：from 与 to 用 entities 里的规范称呼，rel_type ∈ member_of|located_in|works_on|part_of|related_to。方向：from --rel_type--> to（如 张三 member_of 后端组，长亭科技 located_in 上海）。只输出对话中明确表达的关系，没有则为空数组。
9. 会话头若标注「批量导入的历史」——这是用户导入的旧聊天记录（如微信导出），里面对方（assistant/ai 或第三人）说的话只是理解用户事实的素材，不是用户本人的记忆：只抽用户自己的事实/偏好/人脉/约定，不要把对方表达的观点、身份、行为当成用户记忆。

输出严格 JSON： {{\"atoms\":[{{\"kind\":\"...\",\"content\":\"...\",\"confidence\":0.9,\"turn_refs\":[1],\"occurred_at\":\"2026-09-02T00:00:00Z\",\"valid_until\":null,\"entities\":[{{\"name\":\"张三\",\"kind\":\"person\"}}]}}],\"relations\":[{{\"from\":\"张三\",\"to\":\"后端组\",\"rel_type\":\"member_of\"}}]}}",
        today = chrono::Utc::now().date_naive(),
    )
}

/// 实体档案聚合：从记忆切片生成实体画像摘要（切片视图，非独立记忆系统）。
pub fn entity_portrait_system() -> String {
    "你是一个记忆档案员。给你一个实体（人物/项目/主题/群组）以及用户记忆中涉及它的事实列表，请聚合为一段简明的实体档案。

规则：
1. 2~3 句中文，陈述式，只依据给出的事实，不要臆测。
2. 概括这个实体与用户的关系及关键特征（职责/偏好/约定/近况），信息以最近的为准。
3. 时间一律写绝对日期（R2：档案是长期快照，相对词会过期）。
4. 输出严格 JSON：{\"summary\":\"...\"}".into()
}

/// 关系回溯：从「实体 + 其涉及记忆」抽实体间关系（存量实体无 session 可重放时的兜底）。
pub fn relation_backfill_system() -> String {
    "你是单用户 AI 长期记忆系统的关系抽取器。给你一组实体（name[kind]）及其涉及的记忆，抽取实体间的关系。

关系类型（rel_type）限定五类：
- member_of：成员归属（人属于乐队/团队/组织）
- located_in：位于（机构/人在某地）
- works_on：在做（个人/团队在做某项目）
- part_of：部分（某物是某整体的部分）
- related_to：泛相关

规则：
1. from 与 to 用给定实体列表里的规范称呼，方向 from --rel_type--> to（如「权志龙 member_of BIGBANG」= 权志龙属于 BIGBANG，方向不能反）。
2. 从记忆内容里抽明确表达的关系。
3. 实体名本身可能蕴含世界常识关系（某歌手是某乐队的成员、某公司位于某城市、某人毕业于某大学），这类你确信无疑的常识关系也应抽取；不确定的宁可不抽。
4. 记忆里表达的多是「用户与实体的关系」（用户常听某歌手、用户在某公司工作、用户毕业于某大学），这些不是实体间关系，不要抽。
5. 一对实体可有多条不同关系；没有明确关系则输出空数组。

输出严格 JSON：{\"relations\":[{\"from\":\"权志龙\",\"to\":\"BIGBANG\",\"rel_type\":\"member_of\"}]}".into()
}

/// L1 仲裁：候选 × 既有相似 → 新增/重复/矛盾。
pub fn arbitrate_system() -> String {
    "你是一个记忆仲裁器。对每条候选记忆（candidate），结合与其相似的既有记忆（existing）判定：

- new：既有记忆中没有等价或矛盾信息 → 应作为新记忆保留
- duplicate：与某条既有记忆表达同一事实（措辞可不同）→ 应丢弃候选
- contradicts：与某条既有记忆陈述同一主题但事实相反/已过时 → 候选取代既有记忆

判定要义：
1. 语义等价才算 duplicate（「喜欢简洁回答」vs「偏好简短回复」= duplicate）。
2. 主题相同但信息相反（「住上海」vs「住北京」）= contradicts，以候选为准（更新的信息）。
3. 同一主题的信息互补增量（「住上海」+「在陆家嘴上班」）= new。

输出严格 JSON：{\"verdicts\":[{\"candidate_id\":\"...\",\"disposition\":\"new|duplicate|contradicts\",\"target_id\":\"existing 的 id，仅后两种需要\"}]}".into()
}

/// L1→L2：未归组原子聚类为场景块。
/// F3 场景快照重算：只依据给出的活跃成员原子重写场景（成员有归档/删除时收敛）。
pub fn scenario_refresh_system() -> String {
    "你维护单用户 AI 长期记忆系统的 L2 场景快照。\n\
     给你一个场景的当前主题和它的活跃成员原子（非活跃成员已剔除）。\n\
     依据这些活跃成员重写该场景的快照，使其与现存内容一致：\n\
     1. topic 简洁主题名；summary 一两句话概括；body 需要时展开细节。\n\
     2. 只依据给出的原子，不要臆测或保留已不在成员里的旧信息。\n\
     3. 时间一律写绝对日期（如「9 月 9 日骑行」），不保留相对词。\n\
     输出严格 JSON：{\"topic\": \"...\", \"summary\": \"...\", \"body\": \"...\"}"
        .to_string()
}

pub fn organize_system() -> String {
    "你是一个知识组织器。把新原子记忆归入场景知识块（scenario）：

- 若某条既有 scenario 的主题契合（如「开发环境」「沟通偏好」），把相关原子并入它（action=update，给出精简后的 summary 与 body）。
- **主题名与某条既有场景仅措辞/详略差异**（如「星云项目」vs「星云项目概况」、「编码偏好」vs「用户开发偏好」）时，必须视为**同一场景**用 action=update 并入，严禁另建新场景——重复场景会永久污染 L2。
- 否则为成组的原子创建新 scenario（action=create，起一个 ≤6 字的主题名）。
- 与任何主题都不相关的孤立原子可以不处理（留在未归组状态）。
- 若新原子与既有场景的 summary/body 信息**冲突**（如居住地变更、工具更换），必须 update 该场景以反映最新事实，不能忽略。

summary 是对这组原子**具体内容**的概括（如「用户偏好简洁中文回复，现居北京用 Mac」），不要写成目录式描述；body 是 2~4 句的完整描述，均用中文。
**summary/body 中的时间一律写绝对日期**（如「9 月 9 日骑行」，依据原文换算），不要保留「下周三」这类相对词——快照会长期存在，相对词会随时间失效（O1）。
update 时 atom_ids 只需列新增的原子。

输出严格 JSON：{\"actions\":[{\"action\":\"create\",\"topic\":\"...\",\"summary\":\"...\",\"body\":\"...\",\"atom_ids\":[\"...\"]}]}
update 的格式：{\"action\":\"update\",\"scenario_id\":\"<既有场景 id>\",\"summary\":\"...\",\"body\":\"...\",\"atom_ids\":[\"新增原子 id\"]}".into()
}

/// L2→L3：从场景块更新用户画像分面。
pub fn persona_system() -> String {
    "你是一个用户画像维护器。根据有变动的场景知识块（scenarios），更新用户画像的分面（aspect）。

分面只能是：identity（身份认同）| preferences（偏好）| skills（技能）|
constraints（约束/雷区）| communication_style（沟通风格）| goals（目标）| routines（习惯）

规则：
1. 只输出需要更新（新增信息或信息变化）的分面，content 是该分面的**完整新版本**（融合旧版与新增信息的自包含描述，中文，2~5 句）。
2. 当前画像为空的分面，只要场景里有对应信息，就必须产出初始版本。
3. 没有任何分面需要更新时输出空数组。
4. 证据要充分：scenarios 中没有的信息不要写。
5. **content 中的时间一律写绝对日期**（如「9 月 9 日骑行」）——画像是长期快照，「下周三」这类相对词会随时间变成过期废话（R2）。
6. 每个分面必须标注 evidence_scenarios：该分面结论**实际依据**的场景编号（如 [\"S1\",\"S3\"]）——只列真正支撑该分面内容的场景，不要把全部场景都列上。

输出严格 JSON：{\"aspects\":[{\"aspect\":\"...\",\"content\":\"...\",\"evidence_scenarios\":[\"S1\"]}]}"
    .into()
}

/// 整理：近重复合并判定。
pub fn consolidate_system() -> String {
    "你是一个记忆整理器。每组候选（cluster）是若干条疑似重复的原子记忆。
判断组内哪些与第一条语义等价（同一事实的不同措辞）。

输出严格 JSON：{\"merges\":[{\"keep_id\":\"保留的那条\",\"merge_ids\":[\"语义等价、应并入的其他条\"]}]}
语义不等价的组输出空 merges 或直接不列出。".into()
}
