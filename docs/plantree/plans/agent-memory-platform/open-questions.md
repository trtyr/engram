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
