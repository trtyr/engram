########## Stage 1: 前端构建 ##########
FROM node:22-slim AS web-build
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

########## Stage 2: 后端构建（cargo-chef 分层缓存）##########
FROM lukemathwalker/cargo-chef:latest-rust-1.97-slim AS chef
WORKDIR /build

FROM chef AS planner
COPY server/ .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# 依赖层（源码不变时命中缓存）
RUN cargo chef cook --release --recipe-path recipe.json
COPY server/ .
RUN cargo build --release -p agent-memory-api

########## Stage 3: 运行时 ##########
# debian-slim 而非 distroless：Phase 5 需要安装 codegraph（Node runtime）
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/agent-memory-server /usr/local/bin/agent-memory-server

# 运行时数据卷（uploads / wiki-sources / codegraph）
RUN mkdir -p /app/data
VOLUME /app/data

ENV AGENT_MEMORY_PORT=8080
EXPOSE 8080

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=5 \
    CMD curl -fsS http://127.0.0.1:${AGENT_MEMORY_PORT}/ready || exit 1

ENTRYPOINT ["agent-memory-server"]
