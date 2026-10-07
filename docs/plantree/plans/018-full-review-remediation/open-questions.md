# P018 · Open Questions（等拍板，只留未决）

> 来源标注路别；file:line 证据见 evidence/ 下对应 JSON 与 [code-review-2026-10-07.md](evidence/code-review-2026-10-07.md)。

| # | 问题 | 来源 | 拍板点 |
|---|---|---|---|
| Q001 | **散点 env 绕过集中加载**：检索阈值 VEC_FALLBACK_*（search/hybrid.rs:27-41）、EMBED_QUERY_INSTRUCTION（core/memory.rs:143-147）、CG_LAYOUT_ITERS（cg-bridge/precompute.rs:11-17）、LOG_RETAIN_*（api/logging.rs:268-275）、MCP_ALLOWED_HOSTS（mcp/server.rs:246-256）、JOB_CONCURRENCY（api/main.rs:112-114）均现场 `std::env::var`，入口分散无总表 | config ⑧ | 阈值/迭代类迁配置文件或集中声明 vs 维持现状 |
| Q002 | **EMBEDDING_DIMENSIONS 死参数**：配任何非 1024 值一律拒启（api/config.rs:24-31/67-86），可配空间为零——错误文案虽含迁移指引但参数本身无意义 | config ⑦ | 删参数 vs 真支持维度迁移（后者要动向量列） |
| Q003 | **engramctl 弱默认注入**：~/.engram/.env 缺项时静默注入 ADMIN_PASSWORD=dev-pw、MASTER_KEY="ab"×32（scripts/engramctl.py:218-228）——便利优先于安全 | config ③ | 改显式必填/随机生成 vs 保持本地开发便利 |
| Q004 | **guard.rs 七连 require_\* 逐字复制**（mcp/guard.rs:42-115）+ **AppState 三连 master_key→cipher 推导复制**（core/state.rs:53-90）：一次权限/密钥语义改动要改多处 | coupling 微重复簇①② | 收敛为单实现 vs 保持直白复制 |
| Q005 | **DOMAIN_TOOLS 三方手工同步**（mcp/dispatch.rs:204-227/264-268）：12 域名硬编码清单与域模块、scope 名三方手工同步，新增/改名一个域至少改 4 处（dispatch 清单、guard 函数、域模块、api 路由三件套）；跨域主键命名不一（id/asset_id/source_uri）靠报错文案兜底 | coupling 内容耦合 | 表驱动化/生成 vs 维持手工（MCP 面改动需过 golden 快照） |
| Q006 | **ATOM_MAX_CHARS=500 双写常量**：distill/extract_model.rs:15-17 与 core/memory.rs:165-167 各一份，靠注释「两处人工同步」维系，类型系统无法发现漂移 | coupling 远距离动作 | 集中到单一 crate 导出 vs 注释声明够用 |
| Q007 | **secrets 过滤机制化**：日志写侧无 secrets 过滤（靠纪律）；LLM 失败日志打响应体前 500 字符（llm/chat.rs:170-175）；主密钥错误信息含前 8 hex（api/main.rs:115-124） | logging 最弱① | 做写侧过滤器/截断策略 vs 维持纪律 |
| Q008 | **「文档即数据」死引用税**：AGENTS.md 的 UUID 文档清单对无活服务连接的代理是不可解析死引用（AGENTS.md:3-6/18-64）；本地 docs/plantree/ 决策文档仅 AGENTS.md:77 一笔带过 | aifriendly 域2 | 索引段补 fallback 说明（离线时读哪里）vs 不加 |
