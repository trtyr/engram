# Phase 5 — CodeGraph 桥

**目标**：不写一行解析器，把 codegraph 现成能力接进平台：注册项目、建索引、稳定代理查询。Docker 内可用。

## 前置

Phase 1。

## 交付物

### cg-bridge crate

- [ ] CLI 封装：spawn + `--json` 解析 + 版本探测（不匹配→项目标 version_mismatch）
- [ ] 超时矩阵：init/index 10min、sync 60s、query 30s；kill + timeout 错误归一
- [ ] git URL 注册 → 平台 clone（depth 1）到 `data/codegraph/<id>/`；本地路径注册校验存在
- [ ] 状态机：registered→indexing→ready/error；sync 增量；stats 从 status 解析缓存
- [ ] 查询代理五种 kind 的 DTO 归一层（上游 schema 变更单点适配）+ 大输出截断摘要

### Docker 集成

- [ ] 镜像内安装 codegraph（pin 版本，Q3 落定安装方式）；`data/codegraph` 卷声明
- [ ] README 记录上游版本与升级流程（升级 = 改 pin + 重新探测，全项目 version_mismatch 人工重扫）

### API

- [ ] projects CRUD + sync + status；`POST /codegraph/query` 五 kind

## 出口标准

1. e2e（evidence 留档）：compose 栈内注册一个真实 Rust 仓库（如本仓库自身）→ indexing → ready → explore「蒸馏管道如何触发」返回相关符号与调用路径；callers/impact 各验一例
2. 指向不存在路径/坏 git URL → error 状态 + 可读错误；CLI 超时路径单测（模拟挂起子进程）
3. 版本不匹配注入测试 → 查询拒绝且状态明确，不产生半解析结果
4. 上游 JSON 缺字段样例 → 归一层容错（明确错误或降级字段），不 panic
5. Phase 0/1 出口标准依然全绿

## 关联

- 设计：[codegraph-bridge](../topics/codegraph-bridge.md)
- 决策：D0005
- 风险：R3、R8
