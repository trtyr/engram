# 老代码功能移除的完整性检查清单（D1，2026-10-02 P008 人审移除实战）

> 触发：P008-T001/T002 移除 wiki 人审机制时，两轮门禁+独立审计才抓全残留。
> 价值：**功能移除的漏网是系统性盲区**——不是「忘了改」而是「搜索面天然不全」。

## 第一层：调用面（初次搜索就能找到的）

按符号名全仓 grep：`fn 名`、`struct 名`、`action 字面量`、`端点路径`。
本轮命中：`reviews`/`review_resolve`/`apply_proposal` 三 action、
4 HTTP 端点、前端三组件。

## 第二层：派生面（同义词与跨 crate 路径，最容易漏）

- **跨 crate 全路径引用**：`engram_wiki_engine::review::X` 这种路径
  **不带 `crate::review`**，按模块名搜会漏。
  搜 `::review::`、`use ...review`、re-export 行。
- **同名不同义的干扰项**：study 域的 SRS `reviews`（复习队列）
  与 wiki 人审 `reviews` 同字面量——全局删除时**必须先判定域归属**，
  否则误删正常功能。
- **间接调用点**：`create_lint_items()` 是调用点，真正的 `INSERT` 在它实现里（review.rs:95）。
  按「写入点」搜 INSERT 会漏掉调用点，反之亦然——**调用点与实现点要分别扫**。

## 第三层：行为面（门禁才抓得到的）

- **测试文件逐个扫**：本轮漏了 `mcp_wiki_curation_test.rs`（连 `mcp_test`/`wiki_test` 都清了，
  这个第三个文件没人想起）→ workspace 门禁 EXIT=101 抓出。
  **教训：清测试要 `grep -rln <符号> --include='*_test.rs'` 拿全清单**，
  不能凭印象点几个文件。
- **快照/清单类**：golden（`UPDATE_GOLDEN=1` 自动重生）、openapi 手维护清单（**要手动删+按字典序**）、
  代码里的硬断言（`mcp_test.rs:762` 的 `28 个操作`）。
- **前端产物**：`dist` 是编译期生成的，删完源码要**重新 build 再 grep 产物**验证零残留。

## 第四层：语义面（审计最挑的——「代码删了但话还在」）

- **陈旧 doc 注释撒谎**：`lint_deep.rs` 头注释仍写「结果写入 wiki_review_items，走现有人审队列」，
  实际代码早就不写了 → 审计判为「documentation lies」。
  **规矩：删代码行时，同一 hunk 内的注释必须一起改；跨文件的注释要单独扫一遍关键词。**
- **功能残留**：`ingest/generate.rs` 里仍有一段 `INSERT INTO wiki_review_items`（失败落 flag）——
  删除 action 层时完全没想到「另一条链也写这张表」。
  **规矩：以「表名 / 存储对象」为关键词反查全部写入点，比按功能名搜可靠。**
- **状态机残留**：`cascade_dismiss` 两处 best-effort 调用（删页/删源时清审查项）。
- **文案残留**：MCP 工具组名仍是「织入」、描述里的「产出入人审队列」。

## 第五层：文档面（plan tree 自身）

- **roadmap 的 Done 与 Next 不能同时非空**：本轮把 7 个任务标 Done 却没清 Next 段，
  审计直接判「终态 commit 制造了自相矛盾状态」。
  **规矩：终态 commit 必须一次性清 In Progress/Next**，
  且 commit message 的「终态」断言要能被 diff 证明。

## 一句话心法

> 删功能不是删代码，是**删一个概念的全部投影**：调用点、实现点、跨 crate 路径、
> 写同一张表的所有链、注释、文案、测试、快照、确定性清单、plan tree 状态段。
> 逐层扫，别靠印象。
