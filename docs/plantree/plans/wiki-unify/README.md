# wiki-unify（Knowledge + Wiki 合并为「Wiki」域）

## Scope

用户判断（2026-09-02）：「wiki 和知识库其实是一个东西」——Knowledge（原料 RAG 库）与 Wiki（AI 织的知识网）本是同一条知识管线的两层，被硬拆成两个域、两套 API、两条原料管线，导致「资料该丢哪」的困惑。

合并方向（用户三拍板）：
1. **前后端彻底合一**（后端一个 /wiki 域，前端一个 Wiki 页）
2. **命名保留 Wiki**
3. **原料→成品自动织**（丢资料 → 存原料可 RAG 检索 → 后台自动织成页面/链接）

## Authority

用户方向（2026-09-02）：
- 「我觉得 wiki 和知识库其实是一个东西」
- 拍板：彻底合一 / 保留 Wiki / 自动织

## File Map

| 文件 | 角色 |
|---|---|
| [roadmap.md](roadmap.md) | 阶段状态（Done / Next / Deferred） |
| [topics/merge-design.md](topics/merge-design.md) | 合并方案：数据模型 / API 迁移 / 自动织管线 / 前端融合 |
| [open-questions.md](open-questions.md) | 未决问题 |

## Reading Path

1. 本 README（scope + authority）
2. [topics/merge-design.md](topics/merge-design.md)（合并方案全景）
3. [open-questions.md](open-questions.md)（动手前要拍的板）
4. [roadmap.md](roadmap.md)（阶段状态）
