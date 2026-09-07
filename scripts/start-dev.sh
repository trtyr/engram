#!/usr/bin/env bash
# 开发机一键起栈（Windows Git Bash / Linux 通用）。
# 背景：2026-09-06 事故——重启服务时漏带 AGENT_MEMORY_MCP_ALLOWED_HOSTS，
# 局域网 MCP 客户端（zcode 等）全部 403 无限重试，表现为「工具调用卡死」。
# rmcp 的 DNS-rebinding 防护默认只放行 loopback；LAN 部署必须显式加白名单。
# 以后重启服务一律用本脚本，不要手搓环境变量。

# MCP Host 白名单（rmcp DNS-rebinding 防护，默认仅 loopback）。
# 每新增一个访问地址（LAN IP / Tailscale IP / 域名）都要加进来——不带端口的条目
# 匹配该主机任意端口。症状备忘：漏加 = 该地址来的 MCP 客户端 403 无限重试「连接不上」，
# 但前端能开（静态资源不走此检查）。空列表 = 放行所有 Host（不建议公网用）。

# 任何端口/地址变更后记得同步白名单。
set -euo pipefail
cd "$(dirname "$0")/../server"

AGENT_MEMORY_DATABASE_URL='postgres://127.0.0.1:5432/engram' \
AGENT_MEMORY_PORT=8080 \
AGENT_MEMORY_ADMIN_PASSWORD='admin123' \
AGENT_MEMORY_MASTER_KEY="$(printf 'ab%.0s' {1..32})" \
AGENT_MEMORY_DATA_DIR=/tmp/am-data \
AGENT_MEMORY_MCP_ALLOWED_HOSTS='192.168.3.92,100.80.65.64,localhost,127.0.0.1' \
RUST_LOG=info \
  ./target/debug/engram-server.exe
