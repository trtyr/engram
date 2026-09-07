#!/usr/bin/env bash
# 开发机一键起栈（Windows Git Bash / Linux 通用）。
# 背景：2026-09-06 事故——重启服务时漏带 AGENT_MEMORY_MCP_ALLOWED_HOSTS，
# 局域网 MCP 客户端（zcode 等）全部 403 无限重试，表现为「工具调用卡死」。
# rmcp 的 DNS-rebinding 防护默认只放行 loopback；LAN 部署必须显式加白名单。
# 以后重启服务一律用本脚本，不要手搓环境变量。
set -euo pipefail
cd "$(dirname "$0")/../server"

# 任何端口/地址变更后记得同步白名单（含不带端口与带端口两种 Host 形态）
AGENT_MEMORY_DATABASE_URL='postgres://127.0.0.1:5432/engram' \
AGENT_MEMORY_PORT=8080 \
AGENT_MEMORY_ADMIN_PASSWORD='admin123' \
AGENT_MEMORY_MASTER_KEY="$(printf 'ab%.0s' {1..32})" \
AGENT_MEMORY_DATA_DIR=/tmp/am-data \
AGENT_MEMORY_MCP_ALLOWED_HOSTS='192.168.3.92,192.168.3.92:8080,localhost,127.0.0.1,127.0.0.1:8080' \
RUST_LOG=info \
  ./target/debug/engram-server.exe
