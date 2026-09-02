# 当前状态（2026-09-01 验证基线）

> 2026-08-30 初始化后的首次全面更新。

## 一句话状态

vitest 37 / oxlint 0 警告 / tsc 0 / build 0 / journey PASS（含收尾自清）。
IA 定稿为七页：圈子拆独立页，Wiki+Knowledge 合并成一个 Wiki 页。

## 2026-08-30 基线以来的前端大事记

1. **记忆星系 → 圈子 → 独立页**：先以 tab 落地（8bfd330），改名圈子（3e9f95b），
   最终拆独立页 /circle（36342e8）——一坐标系脱离一架梯子
2. **人审队列 tab**（d7c7fdf）：通过/取代/丢弃 + 批量
3. **re-embed 横幅**（ae4daad）：向量缺失诊断与修复入口
4. **编辑能力 UI**（534d701）：原子双击编辑/敏感开关/历史抽屉 + 画像编辑钉住 + 实体摘要手编
5. **画像三件套**（1d9ee28）：右滑历史抽屉 + 句子级 LCS diff + 证据链跳场景
6. **deep 清空 UI**（bfa2e3f 部分）：危险区确认短语门禁
7. **登录态根修**（b5ac042）：探活 401-only（5xx 不再误杀）
8. **e2e 自清**（a7ae4f4）：journey 收尾清 agent/实体/key
9. **测试隔离**（399deab）：E2E_BASE 必填拒跑 + 一次性栈脚本 + 快照差分自清
10. **设置节律 tab**（c1f877f）：cron 心跳三态 / 积压年龄 / crontab 安装向导 / 节律事件流
11. **Wiki+Knowledge 合并**（2026-09-02）：/knowledge 并入 /wiki 前缀，前端融合一个 Wiki 页（文档 tab 默认，接管上传/URL/阅读/检索），删 Knowledge.tsx + 侧栏「知识库」项；图谱 Obsidian 化（hover 邻居高亮/拖拽/缩放/边权重/位置缓存）

## 已知前端未了项

- 初始 bundle 预算 350kB 内（当前 282kB，sigma/mermaid/cytoscape 均在 lazy chunk）
