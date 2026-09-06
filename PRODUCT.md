# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

- 人类用户：平台所有者本人（单用户），通过 Web 控制台管理、浏览、治理五类记忆资产。
- AI 用户：持有 API key（scopes 限定）经 HTTP API / MCP 操纵平台——写会话、触发蒸馏、摄取知识、编译 Wiki、查图谱、读写技能库。

## Product Purpose

单用户 AI 长期记忆平台。核心理念「平台即工具」：平台对外暴露 HTTP API，AI（或人）拿着 API 操纵平台；人通过 Web UI 管理浏览。五类长期记忆资产：Chat Memory（L0→L3 分层蒸馏，全程可溯源）、Wiki（文档知识：文档/URL 摄取→混合检索 + LLM 增量维护的互链知识库）、CodeGraph（代码知识图谱）、项目记忆（跨会话工作线；dev 模板五分类含部署）、Skills（SKILL.md 形态的可复用指令包：导入/检索/版本回滚）。成功 = 所有者与其 AI 能长期信赖的、可溯源的记忆系统。

## Positioning

邻近产品难以照抄的机制：蒸馏链每层记录 prompt_version 可归因回放；PG 任务队列承载一切长操作；混合检索（jieba FTS + pgvector ANN + RRF）中文友好；单二进制 + 内嵌 SPA 单端口交付。

## Operating Context

本地/Docker 单机部署（compose：pgvector/pg17 + 单镜像）。管理员密码登录（无多租户）。LLM provider 在 Settings 内注册（密钥 AES-GCM 加密落库）。长操作异步走任务队列，页面需要表达 pending/running/succeeded/failed/dead 生命周期。

## Capabilities and Constraints

- 十个页面：Dashboard / Memory / Circle / Wiki / CodeGraph / Projects / Skills / Jobs / MCP / Settings + 登录。
- 系统迁移（2026-09-06 定）：`GET /migrate/export`（迁移包 JSON：五域全部业务数据）+
  `POST /migrate/import`（冲突跳过合并语义，可重复执行）+
  `POST /migrate/pull`（B 机一键拉取 A 机，认证 = A 机管理员密码仅请求期使用）；
  派生数据（embedding/tsv/codegraph 索引）不迁移，导入端重建。全部 admin-only。
- MCP 工具面（2026-09-05 定，45 工具）：工具描述是发现通道——`skills_list` / `project_list` 的描述动态织入库内资产清单（slug+description / 项目名+状态），与 tools/list 同源；控制台 MCP 页点击工具行展开完整描述 + 参数 Schema（管理台所见即 AI 所得）。
- Skill = 文件夹（2026-09-05 定）：SKILL.md 本体 + 附属文件（scripts/、references/…）按相对路径寻址。文件是「内容」不是「文件系统位置」——云端只存内容永不执行，客户端取走后在本地跑。三种消费形态按需选：① 纯文本 MCP 读（skills_get/file_get）② 单文件 HTTP 直下 `GET /skills/{slug}/file?path=…&raw=1` ③ 整包 `GET /skills/{slug}/bundle`（zip：SKILL.md+全部文件）；工具描述内嵌形态选择指南。
- 后端 HTTP API（104 路径，OpenAPI 权威）已定型。
- 技术栈保持：React 19 + TypeScript + Vite 8 + Tailwind 4 + pnpm（radix/shadcn 基件可用可弃）。
- e2e（Playwright journey）断言语义保持，选择器允许随新 DOM 同步更新。
- 无营销面：纯控制台（Operate 模式），无注册/计费/多语言诉求；界面语言中文。

## Brand Commitments

- 命名：**Engram**（2026-08-30 定）——engram＝记忆痕迹，神经科学中记忆在脑中留下的物理印记。UI/标题/品牌位用 Engram；仓库随品牌改名为 trtyr/engram（2026-09-04，原 agent-memory），crate/二进制 engram-*。
- 视觉世界：墨白正统（Vercel/Geist 系，见 DESIGN.md）。旧视觉（Geist+蓝+shadcn 默认）为反参照，已整体替换。

## Evidence on Hand

- 仓库根 `docs/`：完整全栈档案（overview/architecture/api/data-model 等）。
- 审计阶段将以本地栈（本机 PG + 种子数据）对全部页面截图取证。
- 无营销素材、无客户证言——未来工作不得虚构。

## Product Principles

1. **操作优先**：这是所有者每天用的控制台，扫读、一致性、真实使用场景高于表达欲。
2. **信任源于可溯源**：蒸馏层级、任务生命周期、原料→产物链条要在界面上可见可查。
3. **单用户的亲密感**：没有多租户噪音、没有 onboarding 仪式；像个人工具，不像 SaaS 后台。
4. **AI 是一等公民**：API 契约（OpenAPI 类型同步）不可破；UI 是治理面不是唯一入口。
5. **异步是常态**：一切长操作走队列，界面必须优雅表达等待、失败与重试。

## Accessibility & Inclusion

未确立产品特定标准；遵循 web 通用无障碍实践（对比度、键盘可达）作为工程底线。
