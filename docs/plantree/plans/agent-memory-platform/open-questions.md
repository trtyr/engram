# Open Questions — 未决问题

> 问题不是任务。每个问题标注影响阶段与当前倾向；拍板后移入 decisions 并在各 phase 文件更新引用。

## Q3 codegraph 版本 pin 与镜像内安装方式（影响 P0/P5）

npm 全局装（自带 runtime，镜像大）vs 官方 install 脚本 bundle。倾向 npm pin 版本；P5 开工前在容器内验证安装与运行。

## Q4 蒸馏触发默认参数（影响 P2）

debounce 窗口（暂定 30s）、最小聚合会话数、consolidate 周期。做成可配，默认值 P2 用真实使用校准。

## Q5 LLM 产出语言（影响 P2/P4）

跟随源语言 vs 固定中文 vs per-purpose 可配。倾向「跟随源、默认中文」写进提示词约定。

## Q6 管理员会话 token 形态（影响 P1/P6）

JWT（无状态、登出麻烦）vs opaque token + PG 表（可吊销）。单用户场景倾向后者，简单可控。

## Q7 compose 镜像拆分（影响 P5/P7）

单镜像（含 codegraph，大而全）vs app + codegraph sidecar 两镜像。倾向先单镜像，体积实测后再定（R8）。

（Q1 中文检索已决 → D0009；Q2 embedding 已决 → D0010）

## 初始化审计补充（2026-08-20，/init 全量重新初始化记录）

> 不属于原计划问题，是初始化审计发现、值得后续处理的开放项。拍板后同样移入 decisions。

- ~~**Q8 依赖漏洞**~~（**已解决 2026-08-23**）：lopdf 0.34（RUSTSEC-2026-0187，high 7.5）——
  升级 pdf-extract 0.8.2 → 0.12.0（内部 lopdf 0.42），audit 复扫漏洞消失。
  余下：tokio-tar（仅 testcontainers dev）、rsa（lockfile 孤儿，无引用方），
  以及新增传递依赖 unmaintained 警告 ttf-parser（pdf-extract 0.12 引入，可接受权衡）。
- ~~**Q9 api→域 crate 直连 vs core 边界**~~（**已解决 2026-08-23，路径 A**）：
  新建底层 `parsing` crate 解开 wiki-engine→core 依赖环；core 新增 wiki/codegraph 门面；
  api 不再 import wiki-engine/cg-bridge（grep 零匹配）。module-map/AGENTS.md 已同步。
- ~~**Q10 zustand 未使用**~~（**已解决 2026-08-23**）：从 package.json/lockfile 移除
  （src 零引用），AGENTS.md 约定与 D0006 加注记同步。
- **Q11 test-results/ 跟踪**：根目录 `.last-run.json` 曾提交（Playwright 元数据），
  .gitignore 已加 `test-results/`，历史文件已 `git rm --cached` 移出跟踪（文件保留在磁盘）。
