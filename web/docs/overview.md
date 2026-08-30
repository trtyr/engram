# 概览

Engram 控制台：把 agent-memory 的七域能力装进一个可导航、可观测的单页应用。

## 设计世界（一句话）

墨白正统——纯黑白立场 + 1px 发丝线 + 整行反转选中 + Geist/Geist Mono；彩色只留给状态语义
（success/warning/destructive/info）与数据编码；亮暗双主题为同一世界的两种表达。
完整规范见根 [DESIGN.md](../../DESIGN.md)。

## 页面地图（7 域）

| 路由 | 页面 | 核心内容 |
|---|---|---|
| / | Dashboard | 跨域统一检索、5 格统计带（等宽数字）、近期任务、LLM 用量表 |
| /memory | Memory | **管线条（L0会话→L1原子→L2场景→L3画像，签名交互）** + 会话/原子/场景/画像/检索五标签 |
| /knowledge | Knowledge | 文档上传（拖拽+轮询状态机）、分块明细、语义检索 |
| /wiki | Wiki | 页面(Markdown+mermaid+wikilink)/图谱(sigma)/洞察/Lint/提案/源数据/目的 七标签 |
| /codegraph | CodeGraph | 项目注册→索引→结构化查询（面板化结果） |
| /jobs | Jobs | 任务表、事件流水展开、死信 revive |
| /settings | Settings | Provider 管理（真 label 表单）、路由表、API Key、危险区（重加密） |

## 壳能力（2026-08-30 定稿）

- **侧边栏**：208↔60px 收缩（localStorage 持久化）、三分区（首屏/资产域/系统）、
  Jobs 失败计数徽章 + Memory 蒸馏脉冲（10s 轮询）、双主题切换
- **命令面板**：Cmd/Ctrl+K 或 `/` 唤起全局跨域检索（↑↓/Enter/Esc）
- **会话恢复**：401 全局广播回登录页；主题系统跟随 + 手动覆盖 + 跨标签同步；侧栏底部登出（双态可用）
- **移动端**（<768px）：侧栏变顶部横滚导航条，表格横向滚动

## 与后端的关系

同源部署（生产 rust-embed 进后端二进制）；开发走 Vite 代理。
全部数据经 `lib/api.ts` 的 fetch 封装（Bearer 认证）。
细节见 [api.md](api.md) 与根 docs/frontend-backend.md。
