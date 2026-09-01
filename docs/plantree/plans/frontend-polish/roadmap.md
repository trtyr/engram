# Roadmap

## In Progress

（无——R1/R2 已落地，见 Done；R3/R4 待用户排期）

## Next

### R3 移动端与可达性
- 激活导航项 scrollIntoView（切页时保证反转高亮可见）
- 横滚导航渐隐边缘提示（mask-image 渐变）
- skip-to-content 链接（键盘用户跨 8 元素问题）
- iOS 内部滚动手感观察

### R4 杂项（剩余项）
- 路由级 Suspense fallback 骨架化（min-h-40 空白闪一下）

## Done

### R1 P1 修复（2026-08-30 落地）
1. **主题同步**：theme.ts 事件广播（engram-theme-change）+ WikiGraph/MermaidBlock useThemeTick 重渲染。
   验证：页内切主题双截图 magick 对比，图谱 99.99% / mermaid 99.92% 像素翻转。
2. **会话中途失效全局恢复**：api.ts 401 → engram-auth-expired 广播（有 token 才发）→ App 监听回登录页。
   验证：篡改 token 站内导航 2s 内回 /login。
3. **/jobs Accept 分流**（D-001 后端授权）：bearer_auth 中间件顶部 text/html+/jobs → SPA。
   验证：curl 三态（text/html→SPA / 无 Accept→401 / JSON→401）+ 浏览器硬刷新 SPA 挂载。
   server 侧：auth.rs + fmt/clippy/test 100 全绿。

### R2 侧边栏功能补全（2026-08-30 落地，用户钦点）
0. **落地后回归修复**：长内容页（Dashboard）侧栏底部区滚出视口——移动端适配时 `h-screen`→`min-h-screen`
   使桌面失去高度约束，长内容把外层撑高、aside 拉伸、footer 沉底。修复：`md:h-screen`（桌面锁高 +
   main 内部滚动，移动端保持 body 滚动）。6 路由 + 移动端全验证 footer 可见。
0b. **收起态布局修复**（用户视觉反馈「非常奇怪」）：实测两处——头部 brand 容器被 justify-between 挤成
   5px（logo 压扁）、尾部主题按钮溢出侧栏边界 10px（38~66px vs 56px 宽）。修复：收起态头/尾两行
   `md:flex-col md:justify-stretch md:gap-1` 纵向堆叠居中 + brand `shrink-0`。验证：溢出元素 0、
   logo 20px 居中、展开 208px 回归正常、移动端 0px 溢出。
0c. **收起态脉冲点挤位**（用户视觉反馈）：蒸馏脉冲点的 ml-auto 在窄轨里把居中图标推偏。修复：收起态
   点隐藏、图标自身呼吸（engram-pulse 升级 @utility 以支持 md: 前缀变体），rail 标签附「· 蒸馏中」。
   验证：图标居中偏差 0.0px、点 display=none、图标 animation=engram-pulse。
0d. **收起态比例重设计**（用户反馈「图标太小」）：56px 轨 + 16px 图标（占比 0.29）过弱 →
   **60px 轨 + 20px 图标（占比 0.33，Vercel/GitHub rail 范式）**，行高 36px，品牌印记 24px，
   头/尾按钮收起态 p-2 + 图标 20px（ThemeToggle 加 iconClass 透传）。验证：居中 0.0px、
   展开 208/16px 回归不变、26/26、lint 0。
0e. **内容区留白收敛**（用户反馈「留白太多，收缩后更大」）：根因 max-w-6xl(1152) 封顶 + px/py-8——
   收缩释放的宽度全变成居中空白（1512 屏：展开 ~84px/侧 → 收起 ~158px/侧）。改为
   **max-w-[1440px] + 24px 边距**：1512 屏两种状态边距恒为 24px（收起仅 +6px），1920 超大屏
   居中封顶防表格无限拉伸。验证：内容宽 1304(展开)/1440(收起)、26/26、lint 0。
0f. **登出**（用户指出「登进来不能退出，离谱」）：footer 系统区末位加登出按钮——展开态远端角落、
   收起态堆叠底部（36px，hover destructive 语义红）；客户端清 token 回登录页（后端无 logout
   端点，ams_ 随 TTL 过期——MVP 取舍已注释在代码）。验证：双态点击 → /login + token null、
   零溢出、26/26。
0g. **Memory 页五 tab 全面打磨**（用户「细致一点，每个点都细心设计」）：
   - 会话：详情改紧贴行下方的手风琴（不再跳页面底部）；触发蒸馏带 busy + 入队数反馈
     （修 api.ts 202-with-body 被 drop 的库级 bug）；空态文案去 API 黑话
   - 原子：kind 中英对照（偏好/preference 等 8 类）；溯源列（source_refs 计数 + title 明细）；
     置信度 <0.60 黄色警示；5s 无条件轮询改为仅蒸馏中轮询（省闲时请求）；200 条截断提示；
     双击编辑 hover 可见 affordance
   - 场景：卡片补「N 原子 · 相对时间」meta 行；topic 可点击展开 atom_refs 溯源
   - 画像：aspect 中英对照（7 分面）；历史改卡片内展开；回滚加 confirm；版本 chip 修
     bg-white/5 off-token 残留 → border token
   - 检索：busy/空结果 Empty/score 样式统一；L1 命中「查看」跳原子 tab（?tab= 深链扩 search）
   - vitest 28（检索面板新用例）+ e2e journey PASS
0h. **Knowledge 页打磨**（同流程）：
   - 分块预览改紧贴行下的手风琴（原来跳页面底部）；ChunksPanel 移入行内
   - 删除文档加 confirm（连分块嵌入一起删，不可恢复）
   - 上传/URL 摄取带 busy + 三色反馈（摄取中 info / 已入列 success / 失败 destructive），
     摄取中 dropzone 边框 info + 图标脉冲；accept 对齐服务端支持（pdf/docx/html/md/txt）+
     格式提示行；拖拽态由 React state 驱动（弃 classList 直改 DOM）
   - 表格新增「来源」列（URL 摄取 = link 图标 + title 全链；文件 = mime 短码）
   - 检索结果样式统一（标题 chip + #seq + FTS 徽章 + score 右对齐 mono；空命中 Empty）
   - rounded-xl → lg 统一；空态引导文案
   - 实测：手风琴/confirm 取消不删/真文件摄取反馈/双主题截图；28/28 + e2e PASS
0i. **Knowledge 主从版式**（用户「左侧目录右侧查看，像 Wiki 那种；Wiki 也难看」）：
   - 表格式列表 → 左目录（lg:w-80：标题/来源图标/相对时间/处理中脉冲点 + 标题过滤器 +
     N 篇计数）+ 右阅读区（文档头：标题/StatusBadge/来源全链/时间/删除；正文 70ch 成文）
   - 选中态整行反转（Engram 导航语言）；首篇自动选中、删除/过滤后回退首篇无空窗
   - 检索命中行可点开对应文档（document_id 直达）；分块调试信息在段落 title
   - e2e journey 选择器同步（row → 目录按钮 + 阅读区断言）；28/28 + e2e PASS
   - Wiki 主从版式为下一项
0j. **目标 mtfpqhw9-qx7tp4：用户记忆 → 记忆星系**（用户「memory 是 user memory，太浅了，
   应该以用户为中心」）：
   - 全站命名中文化（概览/用户记忆/知识库/Wiki/代码图谱/任务/设置）+ e2e 同步（0a 之前的
     独立提交 707e3a4）
   - 后端实体层 1067a21：迁移 0015（entities + atom_entities，name+kind 活体唯一），
     9 端点（list/graph CRUD attach/detach/merge），蒸馏 prompt 抽实体自动挂链
     （失败仅告警），entity_test 2 用例，api-schema 再生成，workspace 102 测试全绿
   - 前端星系 8bfd330：新首 tab（左列表搜索/类型过滤/密度排序/新建 + 右 sigma 图谱
     懒加载 157kB 独立 chunk）；实体详情（画像/场景/原子时间线/挂摘计数回写/合并 confirm）；
     中心锚点「我」点击跳画像；seed 6 实体 5 边实测零 pageerror
   - e2e journey 适配新默认 tab（reload 后重进会话列表）
0k. **目标 mtft7ahs-4gqfst：记忆模型九项问题全修**（模型分析→小目标集）：
   - 抽取准则放宽 595d5be：「对用户跨会话有用的稳定信息」替代「关于用户本人」——
     社交记忆解锁（张三生日/同事职责）；extract 实体抽取专项测试补审计缺口
   - 实体档案自动生成 2523669：consolidate 步骤 2.5（≥3 密度+摘要滞后触发，
     每轮≤10，LLM 失败仅告警）；首跑逮到并修复 consolidate LATERAL 潜伏 SQL
     bug（真库必 Dead，从未被测过）
   - 检索层补全 0b63dda：search_entities（名字加权 token 匹配）进 /memory/search
     entities 层 + unified entity 域；画像弃 contains 改 jieba 分词打分；
     palette 实体命中直达星系详情（?tab=galaxy&entity=）
   - 记忆域 re-embed ae4daad：reembed_memory job（64/批、archived 不动、全零拒入）
     + status/触发端点 + 原子 tab 修复横幅；修 update_atom 增 needs_review
   - 人审队列 d7c7fdf：人审 tab（通过/取代/丢弃 + 勾选批量）
   - 一致性修缮 3dd210a：done→success 绿；蒸馏口径统一为待蒸馏会话数；
     /jobs 轮询单例化（12s 实测 2 次）；检索 tab 收敛入 palette（深链保留）、
     星系隐藏条带、palette 记忆→原子
   - 全门禁：cargo 107 / vitest 30 / build lint 0 / e2e PASS
0l. **用户四问的仪表盘校正**（条带消失之谜/圈子何用/蒸馏×15 撒谎/会话表太空）：
   - 管线条带退役：计数融进 tab 标签（会话 17 · 原子 9 · 人审 N…，aria-label 保裸名
     测试零改），L0→L3 故事由圈子中心图讲——一行 chrome 归零
   - 星系改名「圈子」（用户自己的比喻：以用户为中心的社交圈）；空态讲清功能
   - 蒸馏脉冲口径再修：processing（真在炼）才脉冲——上一轮改成 pending+processing
     是倒退（e2e 存量 pending 会话导致 ×15 常驻闪烁）；积压改会话页灰字「N 条未蒸馏」
   - 会话表加预览列（首条用户消息为主内容，max-w-96 截断），时间移末列
   - 实测：tabs 带计数+脉冲、条带零残留、预览/积压灰字、e2e PASS 20s
0m. **AI 消费者契约面**（goal mtfx2rg5：记忆域 API 开放给 AI + pi skill 三件套）：
   - amk_ key + memory scope 全旅程 25/25 端点验证（turns 契约/GET context 两处纠错）
   - context_pack 补实体透镜（缺口即修）：AI 冷启动能看到用户世界里都有谁
   - **delete_entity 连带清墓碑**（AI 全旅程实测逮到的潜伏 FK bug：删合并赢家必 503）
   - skill 三件套落 `~/.pi/agent/skills/products/agent-memory/`：SKILL.md（触发/节律）
     + scripts/memory.py（stdlib-only CLI 22 子命令，数组 json/空 id 防呆/HTML
     fallback 护栏）+ references/memory-api.md（全端点+心法，逐例实测；逮到文档
     layers 应为数组、字段是 max_items 不是 limit——脚本同错同修）
   - cargo 109 / api_key_memory_journey + context_pack_carries_entity_lenses 新测试
0n. **测试方 A 档四修 + 平台面开放**（fdfd77d，2026-08-31 深夜 AI 互测第一波）：
   - A1 直写低置信绕过人审（三方打架实锤）→ create_atom 与蒸馏链同规则 <0.55 进人审
   - A4 直写无去重 → 同 kind+内容幂等返回已有原子（近重复仍归 arbitrate）
   - A6 atom_refs 嵌套/扁平混型 → organize UPDATE 的 UNION ALL 整数组当单元素并进
     agg——两侧都展开成标量；存量数据修平
   - 401 区分「已撤销」vs「不存在」（测试方排查建议）
   - **llm scope 上线**（用户拍板：除 amk_ 管理外全暴露）：providers/路由/连通/用量
     8 端点放开；api-keys 管理与 re-encrypt 仍仅管理员
   - 脚本平台面：providers/provider-add/update/delete/test + routing/routing-put +
     jobs/ready + scenarios（A2 全 id、A5 轮次列、B3 同步）；文档补分数语义/幂等/
     needs_review 持久标记/L2 快照语义/base_url 不带 v1
   - cargo 111（+revoked_401 +llm_scope 两测试）；newapi provider 实配（MiniMax-M3
     + bge-m3，连通 chat 1110ms / embed 579ms×1024 维），蒸馏链真 provider 全通
0p. **测试方第二波落地**（c75f668+fc0c193，2026-08-31 深夜——10 议题裁决后当晚交付 7 项）：
   - 时间表达力（议题二）：atoms.occurred_at/valid_until（0016）+ extract prompt
     今天日期锚（相对时间→绝对）+ API 宽容时间反序列化（date-only 与蒸馏层同语义
     ——守夜复测逮到两层打架 422，fc0c193 修）
   - place 实体类型（议题六）：CHECK 扩 5 类 + prompt/CLI/web 配色；判例表进文档
   - session-append（议题一 b）：增量落库（pending-only，agent 补记，30s 防抖共享）
   - 实体级遗忘（议题四）：?forget=true 级联归档→摘链→删实体+墓碑；手动 correction
     superseded_by 取代链补通（犹豫点③关闭）
   - 人审代问（议题三）：context_pack.pending_review ≤5 条
   - no_feedback（议题八/B6）：search+context 不回写热度
   - 文档：confidence 校准锚（犹豫点①）/30s 防抖（②）/L2 快照定位/arbitrate 规则
   - cargo 117（+6）；pi harness 钩子配方（测试方供）记入 testing.md 路线；testing.md
     重写为全能力版本
0q. **第四波收官 + 清场深挖**（3e205df→771a8bc，2026-08-31 用户拍板"全做"）：
   - 用户批准六连：P3 sensitive（0017，检索/pack 默认隐身 + reveal）/ P5 void /
     P11 purge（erase 分权）/ P4 export / R3 画像退休 / P10 新鲜度混排（30 天半衰）
   - R4：export 默认排除 sensitive（隐私出口同权，--include-sensitive 显式含）
   - R3 两档真 bug 修复：stale 名单不进 prompt → 模型静默跳过；措辞升级
     「只保留素材可支撑的表述」——活体清创 identity v7/routines v8 全净
   - 清场深挖：e2e-browser 18 条 journey 会话曾蒸进画像（冲焰场景）——purge
     +场景外科清创+实体 forget；erase/purge 分权活体验证（403/204）
   - galaxy 图四修（f11a4b0）：拖拽真凶=sigma 相机 pan 与节点位移抵消
     （captor.enabled 开关）/分栏独立滚动/图例三行/详情按钮 absolute 钉死
   - 四波互测总账：32 项发现 → 28 落地 + 4 roadmap；cargo 126 / migration 17 /
     CLI 38 子命令 / 文档四件套
   - 待用户：galaxy 手感反馈；roadmap：自动节律钩子（缓做）/erase key 签发/
     定时 full/导入恢复
0r. **终极清空测试四发现**（测试方 seq18，2026-08-31，用户指令「AI 自己把
   系统全部清空」压力测试产物；当日已硬清真空 0/0/0/0/0，原型 SQL：
   TRUNCATE 六表 CASCADE 单事务）：
   - F1 权限倒置：用户侧 AI（memory key）无法清空自己的记忆——erase 分权
     本身对（多 AI 防误毁），缺的是用户侧正当路径 → ② Web 设置页
     「清空记忆库」按钮（管理员二次确认/确认短语）
   - F2 清空非一等操作：四层四种操作无事务无单命令 → ① purge ?deep=true
     （原子归档+实体删+场景解散+persona 清空，需 erase scope，与 F2 授权
     路径绑定）
   - F3 快照层化石（最重要）：源清空后场景/persona 原封不动（stale 按分面
     年龄、素材支撑不排除 archived 成员）→ ③ organize 加「成员全 archived
     场景自动解散」+ consolidate 支撑判定排除 archived
   - F4 敏感不覆盖快照层：青霉素在场景摘要+persona constraints 明文驻留，
     P3/R4 三层防护对 L2/L3 无效（比 R4 严重）→ ④ 敏感归档连带快照重算或
     快照生成跳过 sensitive 溯源
   - 真数据灌入前 ③④ 建议先做（化石+敏感驻留会在真数据重演）
0s. **真数据时代流程约束**（2026-08-31，测试方 seq24/25）：
   - 活体报告必须带实例标识（端口/库指纹/迁移版本号三选一）——"某栈验证过"
     ≠ "当前栈验证过"，消费者会误读（dev 栈 vs 生产栈双实例并存后尤其致命）
   - skill 安装副本（~/.pi/skills，非 git）与 server 版本可能错位——副本
     随 server 版本打版本号 / 一键同步命令（待做）；消费者撞旧副本会误报
     "功能缺失"（--sensitive 实际存在但副本旧）
0t. **AI 主消费者形态确认 + 排期重排建议**（2026-08-31，用户批二验收
   "UI 差不多了"+ 真数据采访后确立）：主消费者是 AI，Web 是辅助观察面。
   - 编辑分权不变，文档写明：AI 编辑=correction 双条留痕，纠错不需要 Web
   - **测试方建议：自动节律钩子（pi extension）提前**——"开场注入+收尾写回"
     的自动化是 AI 消费者使用率与数据质量的倍增器，比任何 Web 打磨都值；
     用户此前拍板缓做，启动与否待用户定。
     2026-09-01 更新：用户拍板**双节律方向**（AI 主动 + cron 定时 + 冲突防御 +
     设置页），已升格为独立规划树 [memory-rhythm](../memory-rhythm/README.md)；
     harness 钩子仍缓做（与 cron 是互补关系，见其 open-questions #3）
   - 卫生项：e2e 每跑一次签一把 key 不回收，key 表已积 20+——journey 收尾
     应自撤（或定期清理）
0u. **deep purge 事故 + 双修**（2026-08-31，测试方冒烟失误清空真数据，
   会话历史重建 32 原子——坑里两个口子当天焊死）：
   - P-A 语义陷阱：deep 与 agent 组合传入 → 400 互斥守卫（agent 在 deep
     下无过滤语义，组合即误导"只清这个 agent"）；CLI 同款本地守卫
   - P-B 直写重建死路：atom-add 原子永不进聚类（organize 只被 candidate
     触发）——extract 空认领也链 organize，批量导入/重建后一次 full 即成形
   - 流程教训（测试方自拟）：破坏性端点测试①非法参数测路径②合法参数只
     对假数据③执行前 echo 请求体人眼过一遍
0v. **P-C 两阶段清空落地**（1472832+ae2d4a9，2026-08-31 收官）：
   - arm（5 分钟冷却）→ token 立即执行 / cancel 后悔药（无需确认短语，
     安全方向免验证）；到期自动执行；job 行即审计链（confirm/by/计数）
   - 0019 迁移 jobs cancelled 态；arm 秒级防抖（取消后可重 arm）
   - 三次清空事故复盘归档：①测试方真参测权限 ②信"已修复"未先探针
     ③开发功能测试打真库——四条规则进 testing.md（含第四条：破坏性
     功能验证必须打一次性栈）
   - 编辑能力整章归档：批一分权/留痕/钉住 + 批二 Web（用户验收"差不多
     了"）+ 测试方四波复核全绿；pi-xiamu 正式 key 在役（v3/v4 已撤）
1. **收缩**：208px ↔ 56px icon 轨，localStorage(engram-sidebar) 持久化，title 提示，动画 200ms。
2. **状态徽章**：useSystemStatus 10s 轮询（页面隐藏跳过）；Jobs 项 failed+dead 计数芯片（收起态角标点）、
   Memory 项蒸馏中脉冲（kind ∈ extract/extract_atoms/arbitrate/organize/consolidate）。
3. **全局检索**：CommandPalette（Cmd/Ctrl+K、/、侧栏按钮）——POST /search，↑↓ 选择 Enter 跳转 Esc 关闭，
   挂载即重置；App 侧 openedAt 派生（路由变化自然关面板，无 effect-setState）。
4. **分区语义**：首屏｜资产域｜系统（mono 小节标签；收起态隐藏标签、分组间发丝线）。
5. 顺带修复：导航双重间距（去掉 space-y 保留 gap）、图标 opacity-90 常量、版本号单一来源
   （vite define 读 server/Cargo.toml workspace version）、跨标签主题同步（storage）、
   系统偏好实时跟随（matchMedia，无手动覆盖时）、移动端检索按钮。
   验证：Playwright 全项（徽章 5/脉冲 1/56px 刷新持久/面板检索命中/Esc/401 恢复/jobs SPA）+
   vitest 26/26（CommandPalette 5 用例新增）+ e2e journey PASS + 移动端 390px 无溢出。
   截图：docs/design/screenshots/r12-*.png（8 张：亮暗×展开收起、面板、图谱/mermaid 双主题、移动端）。

## Deferred

- toast 系统想法（open-questions#2，倾向保持内联反馈）
