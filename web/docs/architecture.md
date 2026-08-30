# 架构

> 2026-08-30 按当日源码。全部行数当日实查。

## 目录结构

```text
web/src/
├── main.tsx                    # 入口（Router 挂载）
├── App.tsx                     # 壳：登录守卫 + 侧边栏（收缩/分区/徽章）+ 命令面板 + 401 恢复
├── index.css                   # Engram token 系统（双主题语义色）+ @utility engram-pulse + 浏览器表面
├── vite-env.d.ts               # __APP_VERSION__ 构建期常量声明
├── lib/
│   ├── api.ts                  # fetch 封装：Bearer、ApiError、401 广播、域类型（Session/Atom/Job/…）
│   ├── api-schema.ts           # OpenAPI 生成（80K，CI 零漂移门禁管）
│   ├── theme.ts                # 主题引擎：useTheme/onThemeChange/useThemeTick + storage/matchMedia 监听
│   ├── status.ts               # useSystemStatus：侧栏徽章 10s 轮询（failed/dead + 蒸馏中）
│   ├── ui.ts                   # 共享样式原语（tableCls/inputCls/tdMono/fmtTime）
│   └── utils.ts                # cn()
├── components/
│   ├── ui-bits.tsx             # Engram 基础件：BrandMark/Card/Tabs(aria-pressed)/StatusBadge(脉冲)/Empty/ErrorBox
│   ├── ui/button.tsx           # Button（solid-ink primary / outline / destructive / ghost）
│   ├── CommandPalette.tsx      # 全局检索覆盖层（Cmd+K、/）
│   ├── ThemeToggle.tsx         # 主题切换（iconClass 透传给窄轨）
│   ├── WikiMarkdown.tsx        # Markdown 渲染：70ch prose、mermaid 主题注入、wikilink 跳转
│   ├── WikiGraph.tsx           # sigma 图谱：度数定尺寸、token 取色、32px 网格底纹、主题跟随
│   ├── InsightsPanel.tsx       # Wiki 洞察卡片
│   └── ReviewQueue.tsx         # Wiki 人审队列
└── features/                   # 七域页（路由级 lazy）
    ├── Login.tsx  Dashboard.tsx  Memory.tsx  Knowledge.tsx
    ├── Wiki.tsx   CodeGraph.tsx  Jobs.tsx    Settings.tsx
    └── *.test.tsx（3 个测试文件）+ components/CommandPalette.test.tsx
```

## 分层规则

```text
features（页面编排，可互相不 import）
   │ 只依赖
   ▼
components（可复用展示件）+ lib（数据与无状态工具）
   │ 只依赖
   ▼
lib/api.ts（唯一出站点）──► 后端 HTTP
```

- 页面间共享走 components/lib，不页面引页面。
- 所有后端调用必须过 `lib/api.ts`（认证、错误体、401 广播在那一处收口）。
- 主题相关的一次性取色组件（sigma/mermaid）必须订阅 `useThemeTick`，禁止挂载时取一次色用到底。

## 关键数据流

```text
登录: Login → api.post(/auth/login) → setToken → App.authed=true → Shell
探活: App mount → GET /jobs?limit=1（复用碰撞路径作探针）
中途失效: api 层 401 → window 事件 engram-auth-expired → App.authed=false → 回 /login
命令面板: Cmd+K → openedAt=locationKey（派生：路由一变面板即关）→ POST /search → 命中跳转
侧栏徽章: useSystemStatus 10s 轮询 /jobs?limit=200 → failed/dead 计数 + 蒸馏族 pending/running
主题: useTheme → localStorage + <html>.dark + engram-theme-change 事件 → 订阅者重渲染
```

## 路由与代码分割

- react-router-dom 7；7 页全部 `React.lazy` 路由级分割。
- 重组件二级 lazy：mermaid（WikiMarkdown 内动态 import）、cytoscape（CodeGraph 内）。
- `__APP_VERSION__` 由 vite.config.ts 构建期从 `server/Cargo.toml` workspace version 注入（版本单一来源）。
