# Open Questions — 未决问题

> 问题不是任务。每个问题标注影响阶段与当前倾向；拍板后移入 decisions 并在各 phase 文件更新引用。

## Q1 中文全文检索方案（影响 P1）

应用层 jieba-rs 预分词（A，倾向）/ zhparser 扩展（B）/ pg_trgm 兜底（C）。
倾向 **A + C 组合**：零扩展依赖，trgm 补子串场景；B 留作后期升级路径。需在 P1 用真实中文语料做召回对比后定案。

## Q2 默认 embedding 模型与维度（影响 P1，schema 定死 vector(N)）

候选：bge-m3（1024，中文强，可自托管/硅基流动）/ text-embedding-3-small（1536，OpenAI）。
**维度一旦写进迁移再改成本高**，P1 开工前必须定。倾向 bge-m3 1024。

## Q3 codegraph 版本 pin 与镜像内安装方式（影响 P0/P5）

npm 全局装（自带 runtime，镜像大）vs 官方 install 脚本 bundle。倾向 npm pin 版本；P0 先验证容器内可装可跑。

## Q4 蒸馏触发默认参数（影响 P2）

debounce 窗口（暂定 30s）、最小聚合会话数、consolidate 周期。做成可配，默认值 P2 用真实使用校准。

## Q5 LLM 产出语言（影响 P2/P4）

跟随源语言 vs 固定中文 vs per-purpose 可配。倾向「跟随源、默认中文」写进提示词约定。

## Q6 管理员会话 token 形态（影响 P1/P6）

JWT（无状态、登出麻烦）vs opaque token + PG 表（可吊销）。单用户场景倾向后者，简单可控。

## Q7 compose 镜像拆分（影响 P5/P7）

单镜像（含 codegraph，大而全）vs app + codegraph sidecar 两镜像。倾向先单镜像，体积实测后再定（R8）。
