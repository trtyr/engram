########## Stage 1: 前端构建 ##########
FROM node:22-slim AS web-build
WORKDIR /build/web
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
WORKDIR /build

FROM chef AS planner
COPY server/ .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# rust-embed 需要编译期可见 web/dist（哪怕占位）
COPY --from=web-build /build/web/dist /web/dist
COPY server/ .
RUN cargo build --release -p engram-api

########## Stage 3: 运行时 ##########
# debian trixie：glibc 2.41 ≥ builder（cargo-chef rust 1.97 基于 trixie），
# 且需要 Node runtime（codegraph pin 安装）+ git
FROM debian:trixie-slim AS runtime
# apt 用清华镜像（稳定直连，不依赖任何代理环境）
RUN sed -i 's|deb.debian.org|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list.d/debian.sources 2>/dev/null; \
    sed -i 's|deb.debian.org|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list 2>/dev/null; \
    apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git npm \
    && rm -rf /var/lib/apt/lists/*
# codegraph CLI pin 版本（D0005；升级需重新验证 JSON 输出格式）
RUN npm install -g @colbymchenry/codegraph@1.5.0 && codegraph version
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
