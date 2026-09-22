# 构建期代理（本机 Clash TUN fake-ip 会污染容器直连出网；BuildKit 对 *_proxy 的
# ARG 自动注入 RUN 环境且不参与层缓存键）。本机构建：deploy/.migrate-env 或
# 手动 --build-arg HTTP_PROXY=http://host.docker.internal:12543
ARG HTTP_PROXY=""
ARG HTTPS_PROXY=""
ARG NO_PROXY="localhost,127.0.0.1,db"

########## Stage 1: 前端构建 ##########
FROM node:22-slim AS web-build
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
WORKDIR /build/web
# npm registry 用 npmmirror（与 apt 清华镜像同理：稳定直连，不依赖代理环境）。
# corepack 不读 npm config——下载 pnpm 本体须用其专用变量。
ENV COREPACK_NPM_REGISTRY=https://registry.npmmirror.com
RUN npm config set registry https://registry.npmmirror.com
# corepack 按 package.json 的 packageManager 字段钉 pnpm 版本
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml web/.npmrc ./
RUN pnpm install --frozen-lockfile
# vite.config.ts 构建期读 server/Cargo.toml 注入版本（__APP_VERSION__ 单一来源）
COPY server/Cargo.toml /build/server/Cargo.toml
COPY web/ ./
RUN pnpm run build

########## Stage 2: 后端构建（cargo-chef 分层缓存）##########
FROM lukemathwalker/cargo-chef:latest-rust-1.97-slim AS chef
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
WORKDIR /build

FROM chef AS planner
COPY server/ .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# crates.io 镜像源（可换：--build-arg CARGO_MIRROR=sparse+https://mirrors.ustc.edu.cn/crates.io-index/）。
# 注：`RUN cat > f <<EOF` 形式的 heredoc 在本项目实测会写出**空文件**（BuildKit 没把 heredoc
# 喂给 cat），于是镜像源配置长期静默失效、构建直连 crates.io——改用单行 printf 并回显自证。
ARG CARGO_MIRROR="sparse+https://rsproxy.cn/index/"
RUN printf '[source.crates-io]\nreplace-with = "mirror"\n[source.mirror]\nregistry = "%s"\n[net]\ngit-fetch-with-cli = true\n' "$CARGO_MIRROR" > /usr/local/cargo/config.toml && cat /usr/local/cargo/config.toml
# rust-embed 需要编译期可见 web/dist（哪怕占位）
COPY --from=web-build /build/web/dist /web/dist
COPY server/ .
# -j 3：8GB 宿主 / ~4GB VM 下 release 编译（thin LTO）并发全开会 OOM 杀掉 buildkit（EOF 中断）
RUN cargo build --release -j 3 -p engram-api

########## Stage 3: 运行时 ##########
# debian trixie：glibc 2.41 ≥ builder（cargo-chef rust 1.97 基于 trixie），
# 且需要 Node runtime（codegraph pin 安装）+ git
FROM debian:trixie-slim AS runtime
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
# 2026-09-13：TUN 虚拟网卡直连——apt 走 debian 官方源（清华镜像在当前网络下不可达）；
# 代理 ARG 保留（历史兼容，默认空不生效）
RUN apt-get update -o Acquire::ForceIPv4=true \
    && apt-get install -y --no-install-recommends ca-certificates curl git npm \
    && rm -rf /var/lib/apt/lists/*
# codegraph CLI pin 版本（D0005；升级需重新验证 JSON 输出格式）
RUN npm config set registry https://registry.npmmirror.com && npm install -g @colbymchenry/codegraph@1.5.0 && codegraph version
WORKDIR /app
COPY --from=builder /build/target/release/engram-server /usr/local/bin/engram-server

# 运行时数据卷（uploads / wiki-sources / codegraph）
RUN mkdir -p /app/data
VOLUME /app/data

ENV AGENT_MEMORY_PORT=8080
EXPOSE 8080

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=5 \
    CMD curl -fsS http://127.0.0.1:${AGENT_MEMORY_PORT}/ready || exit 1

ENTRYPOINT ["engram-server"]
