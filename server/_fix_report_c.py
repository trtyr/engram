import re

def edit(p, pairs, optional=False):
    s = open(p, encoding='utf-8').read()
    for a, b in pairs:
        if a not in s:
            if optional:
                print('SKIP:', p, repr(a[:50]))
                continue
            raise AssertionError(p + ' MISSING: ' + a[:80])
        s = s.replace(a, b, 1)
    open(p, 'w', encoding='utf-8', newline='\n').write(s)
    print('ok', p)

# ---------- D2：storage repo update_doc COALESCE Option 化 ----------
edit('crates/storage/src/repo/project.rs', [
 ("""pub async fn update_doc(
    pool: &PgPool,
    id: Uuid,
    category: &str,
    title: &str,
    content: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_docs SET category = $2, title = $3, content = $4, updated_at = now() """
        + r""""
         WHERE id = $1",""",
  """/// 部分更新（COALESCE）：None 字段保持原值——消除读-改-写并发丢字段窗口
/// （MCP 黑盒测试 D2：两个并发 update 各改不同字段时后者曾整行覆盖前者）。
pub async fn update_doc(
    pool: &PgPool,
    id: Uuid,
    category: Option<&str>,
    title: Option<&str>,
    content: Option<&str>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_docs SET category = COALESCE($2, category), title = COALESCE($3, title), """
        + r""""
         content = COALESCE($4, content), updated_at = now() """
        + r""""
         WHERE id = $1","""),
])
# 修正拼接：上面的三段字符串拼接可能对不上，直接用行级替换兜底
s = open('crates/storage/src/repo/project.rs', encoding='utf-8').read()
if 'COALESCE($2, category)' not in s:
    s = s.replace(
        """pub async fn update_doc(
    pool: &PgPool,
    id: Uuid,
    category: &str,
    title: &str,
    content: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_docs SET category = $2, title = $3, content = $4, updated_at = now() "
         "WHERE id = $1",
    )
    .bind(id)
    .bind(category)
    .bind(title)
    .bind(content)""",
        """/// 部分更新（COALESCE）：None 字段保持原值——消除读-改-写并发丢字段窗口
/// （MCP 黑盒测试 D2：两个并发 update 各改不同字段时后者曾整行覆盖前者）。
pub async fn update_doc(
    pool: &PgPool,
    id: Uuid,
    category: Option<&str>,
    title: Option<&str>,
    content: Option<&str>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_docs SET category = COALESCE($2, category), title = COALESCE($3, title), "
         "content = COALESCE($4, content), updated_at = now() "
         "WHERE id = $1",
    )
    .bind(id)
    .bind(category)
    .bind(title)
    .bind(content)""",
    )
    open('crates/storage/src/repo/project.rs', 'w', encoding='utf-8', newline='\n').write(s)
    print('fallback applied')
else:
    print('primary applied')

# ---------- D1：memory write_session LLM 检查 ----------
s = open('crates/core/src/memory.rs', encoding='utf-8').read()
if 'resolve(engram_llm::types::Purpose::Extract)' not in s.split('pub async fn list_sessions')[0]:
    a = """    let row = repo::insert_session(&self.pool, id, agent, &turns, sensitive, &metadata).await?;

    match distill {"""
    # 该文件实际缩进为 8 空格（impl 块内）
    a = a.replace('\n    match', '\n        match').replace('\n    let row', '\n        let row')
    b = """        let row = repo::insert_session(&self.pool, id, agent, &turns, sensitive, &metadata).await?;

        // LLM 未配置时显式暴露（MCP 黑盒测试 D1：蒸馏静默失败不可接受——
        // manual 最应显式失败；auto 已入库但提示不会蒸馏）
        if distill != "off"
            && let Err(e) = self.registry.resolve(engram_llm::types::Purpose::Extract).await
        {
            let msg = format!(
                "LLM 未配置或不可用（{e}）——蒸馏无法执行。请管理员在「设置 → AI 功能」配置模型后重试"
            );
            if distill == "manual" {
                return Err(MemoryError::LlmNotConfigured(msg));
            }
            tracing::warn!("{msg}（auto 会话已入库，distill_status 保持 pending）");
        }

        match distill {"""
    if a in s:
        s = s.replace(a, b, 1)
        open('crates/core/src/memory.rs', 'w', encoding='utf-8', newline='\n').write(s)
        print('D1 ok')
    else:
        print('D1 SKIP: anchor miss（手工检查）')
else:
    print('D1 already present')

print('batch C done')
