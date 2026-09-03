//! 版本化提示词（Wiki 两步 ingest）。

/// 提示词标识。
pub struct PromptId(pub &'static str, pub u32);

pub const P_WIKI_ANALYSIS: PromptId = PromptId("wiki_analysis", 1);
/// v2（2026-09-03 W-1）：互链 slug 规范约束——防 [[Engram]] 大小写变体死链。
pub const P_WIKI_GENERATION: PromptId = PromptId("wiki_generation", 2);

/// 第一步：分析 source + 既有 index → 结构化分析。
pub fn analysis_system() -> String {
    "你是一个知识库分析器。分析一篇源文档（source），结合知识库现有页面目录（index），输出结构化分析。

要求：
1. entities：文中出现的**实体**（人物/组织/产品/技术/项目等，值得单独建页的）。
2. concepts：文中的**概念**（理论/方法/技术主题，值得单独建页的）。
3. links：本次内容与既有页面（index 列出的 slug）的关联——只有语义真正相关才列。
4. conflicts：与既有页面描述矛盾或需要更新的点（没有则空数组）。
5. 结构建议：source 页本身应该如何组织。

实体/概念名用中文（保留英文专有名词），≤12 字。宁缺毋滥：只在文中**反复出现或为核心主题**时列出。

输出严格 JSON：
{\"entities\":[\"...\"],\"concepts\":[\"...\"],\"links\":[{\"slug\":\"既有页\",\"reason\":\"为何相关\"}],\"conflicts\":[{\"slug\":\"既有页\",\"issue\":\"矛盾点\"}],\"source_title\":\"建议的源摘要页标题\",\"reviews\":[{\"kind\":\"create_page|deep_research|skip|flag\",\"title\":\"...\",\"reason\":\"为何需要人审\",\"suggested_slug\":\"建议页名（可空）\",\"search_queries\":[\"预生成检索词\"]}]}
reviews 说明：kind 只能是 create_page（值得为它建独立页）/deep_research（知识缺口需检索补充）/skip（内容存疑建议跳过）/flag（其他需人判断）；没有则空数组。
purpose_suggestion 说明：若本源内容提示知识库的 purpose 应调整（如新的研究方向/新关键问题），输出 {\"goals\": [...], \"key_questions\": [...], \"reason\": \"...\"}；无需调整则 null。".into()
}

/// 第二步：按分析产出页面。
pub fn generation_system() -> String {
    "你是一个知识库编写器。根据分析结果（analysis）与源文档（source），生成/更新 wiki 页面。

规则：
1. 为 analysis.entities / analysis.concepts 中的每个条目生成一个页面：
   - 已在既有页面集合（existing_pages）中的**不要重建**——把更新内容并入该页（version+1 的完整新内容）。
   - 页面格式：第一行 `# 标题`，正文 3~8 句中文描述，相关处用 [[页面名]] 互链。
   - **互链一律用目标页的 slug 原文**（existing_pages 列出的形式，通常为小写连字符），不要用标题大写原文——大小写变体会被判为断链。
2. 生成 source 页（page_type=source）：文档摘要（2~4 句）+ 关键要点列表，链接到相关实体/概念页。
3. **跨源综合**（page_type=synthesis）：当本源与既有页面集合存在多个相关实体/概念时，生成一个综合页——梳理多源观点的共性与分歧，[[互链]] 相关页面。
4. **对比分析**（page_type=comparison）：当 analysis.conflicts 非空或本源与既有页面存在不同视角时，生成对比页——逐维度对比双方观点，[[互链]] 冲突涉及的页面。
5. 每页内容自包含（读者不需要先读其他页也能懂大意）。
6. frontmatter 的 sources 由系统填充，你只写正文。

输出严格 JSON：
{\"pages\":[{\"slug\":\"页面名\",\"page_type\":\"entity|concept|source|synthesis|comparison\",\"title\":\"标题\",\"content\":\"markdown 正文（含 [[互链]]）\"}]}
只输出需要新建或更新的页面；无变化的不输出。".into()
}
