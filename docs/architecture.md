# 架构（全栈集成视角）

> 模块内部细图各自有档案：[server/docs/architecture.md](../server/docs/architecture.md)、
> [web/docs/architecture.md](../web/docs/architecture.md)。本文写两端的接缝。

## 系统总图

```text
┌────────────────────────── 浏览器 ──────────────────────────┐
│  Engram SPA（web/，React 19）                              │
│  八页（概览/记忆/圈子/Wiki/图谱/项目/任务/设置）+ 壳          │
└───────────────┬────────────────────────────────────────────┘
                │ 同源 fetch（Bearer ams_/amk_）
┌───────────────▼──────────────── engram-server ───────┐
│ axum Router（88 路径 / 113 方法）                           │
│ ├─ 认证层 bearer_auth（/jobs 对 text/html 分流回 SPA）       │
│ ├─ rust-embed：web/dist 静态托管（生产单二进制）             │
│ └─ 域 crate：distill / wiki-engine / cg-bridge / search     │
│    横切：jobs 队列、llm 网关、storage 仓储、parsing 解析     │
└───────┬──────────────────────────────┬─────────────────────┘
        │ sqlx                          │ reqwest（任务化异步）
┌───────▼──────────┐          ┌────────▼──────────┐
│ PostgreSQL+pgvector│         │ 外部 LLM API       │
│ 27 业务表/30 迁移   │         │ （OpenAI 兼容系）   │
└───────────────────┘          └───────────────────┘
                另：cg-bridge 调用外部 codegraph CLI（Node+git）
```

## 蒸馏管线（任务链）

```text
write_session ─▶ extract（L0→L1，含实体抽取）─▶ arbitrate（ANN∪FTS 相似池，LLM 仲裁）
             ─▶ organize（聚类成场景 + 快照收敛：成员归档则解散/重算）
             ─▶ consolidate（近重复合并 + 实体档案生成 + stale 降权）
             ─▶ persona（L2→L3；清退 removed_texts > 手动钉住 > 自动重写）
```

敏感原子（sensitive=true）不进任何摘要输入；归档敏感原子触发防抖的快照刷新
（organize 收敛 + persona 清退），快照层与源数据同生共死。

## 部署断层警示（0s 规则）

push ≠ deploy ≠ 重启生效。生产栈换新二进制必须：重启 + `/ready` 指纹回报
（migration_version + git sha）+ 破坏性契约探针（如 deep+agent → 400）。
历史上三次清空事故里有一次正是"推了没重启"。
