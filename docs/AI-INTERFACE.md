# agent-memory — AI 接口说明

> 本文档是给 AI 客户端（你）的操作手册。agent-memory 是你的长期记忆平台：
> 你通过下面的 HTTP API 读写它。平台由你控制——按需调用，不必等待人类确认。

## 接入

- **Base URL**：环境变量 `AGENT_MEMORY_URL`（如 `http://localhost:8080`）
- **认证**：`Authorization: Bearer <API_KEY>`（管理员在 Settings → API keys 签发，前缀 `amk_`）
- **格式**：JSON；错误统一 `{"error": {"code", "message", "retryable"}}`
- **交互式文档**：`GET /openapi.json`

## 核心工作流

### 1. 会话开始：拉上下文（每次对话第一步）

```
GET /memory/context?query=<本次主题>&budget_items=20&budget_chars=8000
```

返回三层记忆包：`persona`（用户画像）+ `scenarios`（场景块）+ `atoms`（原子事实），
带预算裁剪。`query` 可选（缺省按最近）。

### 2. 会话结束：写入记忆（值得记的才写）

```
POST /memory/sessions
{"agent": "<你的名字>", "distill": "auto",
 "turns": [{"speaker": "user", "text": "..."}, {"speaker": "assistant", "text": "..."}]}
```

`distill: "auto"` 会防抖触发蒸馏（extract → arbitrate → organize → persona），
无需手动触发；蒸馏状态查 `GET /jobs`。矛盾信息会被仲裁（旧的自动 supersede）。

### 3. 随时检索

```
POST /memory/search          {"query": "...", "layers": ["l1","l2","l3"]}
POST /knowledge/search       {"query": "..."}   # 文档知识库
POST /wiki/search            {"query": "..."}   # wiki 页面
```

### 4. 画像与治理

```
GET  /memory/persona                    # 当前画像
GET  /memory/persona/history?aspect=X   # 分面版本历史
POST /memory/persona/rollback           {"aspect": "X", "to_version": N}
GET  /memory/atoms?needs_review=true    # 人审队列
```

### 5. 知识摄取（文档/URL）

```
POST /knowledge/upload       # multipart，字段 file（pdf/docx/html/md）
POST /knowledge/documents    {"url": "https://..."}
GET  /knowledge/documents/{id}   # 状态：pending→parsing→chunking→embedding→ready|failed
```

### 6. Wiki（LLM 维护的知识网络）

```
POST /wiki/ingest            {"title": "...", "text": "..."}   # 两步编译（分析→生成）
GET  /wiki/pages             # 浏览；页面互相 [[互链]]
POST /wiki/lint              # 健康检查（死链/孤儿）
```

### 7. 代码图谱

```
POST /codegraph/projects     {"name": "...", "source_uri": "<git url 或本地路径>"}
POST /codegraph/projects/{id}/index     # 建索引（首次）
POST /codegraph/projects/{id}/query     {"kind": "explore|search|callers|callees|impact", "target": "..."}
```

改代码前 `impact` 查影响面；读代码用 `explore`。

## 任务系统

长操作都是 job（`GET /jobs?status=...` 过滤，`GET /jobs/{id}/events` 看事件流）。
失败任务 `dead` 后可 `POST /jobs/{id}/revive` 重跑。每次 LLM 调用的完整输入输出
都记录在事件里（可归因可回放）。

## 约定

- **预算**：检索都接受 max_items/budget，防上下文爆炸——尊重它
- **写入门槛**：只有跨会话仍成立的信息才值得进 /memory/sessions（宁缺毋滥，
  平台蒸馏时也会过滤）
- **时机**：会话开始 `context`、有新事实随时写、会话结束补完整轮次
