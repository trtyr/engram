# agent-memory

单用户 AI 长期记忆平台。**平台即工具**：平台提供 HTTP API，AI 拿着 API 操纵平台——
存记忆、蒸馏画像、编译 Wiki、查代码图谱；人通过 Web UI 管理浏览。

四类长期记忆资产（短期记忆不在本平台范围）：

| 资产 | 说明 |
|---|---|
| Chat Memory | L0 会话 → L1 原子 → L2 场景 → L3 画像，分层蒸馏，全程可溯源 |
| Knowledge | 文档/URL 摄取 → 分块 → 嵌入 → 混合检索 |
| Wiki | Karpathy 模式：LLM 增量维护的互链知识库 |
| CodeGraph | 代码知识图谱（复用 [codegraph](https://github.com/colbymchenry/codegraph)） |

技术栈：Rust（axum + sqlx）· React 19 + Vite · PostgreSQL 17 + pgvector · Docker。

## 快速启动

```bash
cd deploy
cp .env.example .env
$EDITOR .env          # 修改全部 change-me 项（MASTER_KEY: openssl rand -hex 32）
docker compose up -d --build

curl http://localhost:8080/health   # {"status":"ok"}
```

Web 控制台：http://localhost:8080 （Phase 6 完整可用）。
API 文档：http://localhost:8080/openapi.json

## 开发

- 后端：`cd server && cargo test`（集成测试需要本机 Docker）
- 前端：`cd web && npm run build`
- 规划与路线图：[docs/plantree/](docs/plantree/README.md)
- 仓库操作契约：[AGENTS.md](AGENTS.md)

## 状态

Phase 0（项目地基）已完成；路线图见
[docs/plantree/plans/agent-memory-platform/roadmap.md](docs/plantree/plans/agent-memory-platform/roadmap.md)。
