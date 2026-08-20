# agent-memory

单用户 AI 长期记忆平台。**平台即工具**：平台提供 HTTP API，AI 拿着 API 操纵平台——
存记忆、蒸馏画像、编译 Wiki、查代码图谱；人通过 Web UI 管理浏览。

四类长期记忆资产（短期记忆不在本平台范围）：

| 资产 | 说明 |
|---|---|
| Chat Memory | L0 会话 → L1 原子 → L2 场景 → L3 画像，分层蒸馏，全程可溯源 |
| Knowledge | 文档/URL 摄取 → 分块 → 嵌入 → 混合检索（中文友好） |
| Wiki | Karpathy 模式：LLM 增量维护的互链知识库（两步 ingest + lint） |
| CodeGraph | 代码知识图谱（复用 [codegraph](https://github.com/colbymchenry/codegraph) CLI） |

技术栈：Rust（axum + sqlx）· React 19 + Vite · PostgreSQL 17 + pgvector · Docker 单镜像。

## 快速启动

```bash
cd deploy
cp .env.example .env
$EDITOR .env          # 修改全部 change-me 项（MASTER_KEY: openssl rand -hex 32）
docker compose up -d --build

curl http://localhost:8080/health   # {"status":"ok"}
```

- Web 控制台：http://localhost:8080 （管理员密码 = `.env` 里的 `AGENT_MEMORY_ADMIN_PASSWORD`）
- API 文档：http://localhost:8080/openapi.json
- **AI 客户端接入**：把 [docs/AI-INTERFACE.md](docs/AI-INTERFACE.md) 交给你的 AI（可直接粘贴进系统提示词），
  再在 Settings → API keys 签发一个 key 给它

## 首次配置

1. 打开 Web UI → 登录 → **Settings → providers** 注册 LLM provider
   （OpenAI 兼容网关：base URL + key + chat/embedding 模型各一）
2. 点「测试连通」确认
3. （可选）Settings → routing 按 purpose 配模型路由（缺省用默认 provider）

## 备份与恢复

```bash
deploy/../scripts/backup.sh backup              # → agent-memory-<date>.tar.gz
deploy/../scripts/backup.sh restore <file>      # 栈停止后恢复
```

## 开发

```bash
# 后端（server/；集成测试需要本机 Docker）
cargo test --workspace

# 前端（web/）
npm run build && npx vitest run

# e2e（先起栈，chromium: npx playwright install）
npx playwright test

# OpenAPI 类型再生成（后端 API 变更后）
cargo run -p agent-memory-api --bin openapi-dump > /tmp/openapi.json
npx openapi-typescript /tmp/openapi.json -o src/lib/api-schema.ts
```

- 规划与路线图：[docs/plantree/](docs/plantree/README.md)
- 仓库操作契约：[AGENTS.md](AGENTS.md)

## 状态

Phase 0–7 全部完成（v0.1.0）。路线图与验证证据：
[roadmap](docs/plantree/plans/agent-memory-platform/roadmap.md) ·
[evidence](docs/plantree/plans/agent-memory-platform/evidence/README.md)
