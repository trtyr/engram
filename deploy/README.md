# 部署指南（Docker）

> 仓库里的部署文档是唯一不依赖 engram 自身的文档——部署那一刻 MCP 还没起来。
> 服务跑起来之后，运维/架构文档在 engram projects 域（`/projects` → engram）。

## 前置要求

- 一台 Linux 主机（2 核 4GB 起步；**8GB 以下请设 `CARGO_JOBS=2` 并配 swap**，
  release 编译并发全开会 OOM）
- Docker + compose 插件（`docker compose version` 能跑即可）
- 就这些——PostgreSQL/pgvector、Node、Rust 工具链全在镜像里，宿主机不用装

## 全新部署

```bash
git clone https://github.com/trtyr/engram && cd engram/deploy
cp .env.example .env
```

编辑 `.env`，**必填三项**（全部有 change-me 占位，漏了 compose 会直接拒绝启动）：

| 变量 | 说明 |
|:--|:--|
| `POSTGRES_PASSWORD` | 数据库口令，`openssl rand -hex 16` |
| `AGENT_MEMORY_ADMIN_PASSWORD` | 控制台/MCP 登录口令，`openssl rand -hex 16` |
| `AGENT_MEMORY_MASTER_KEY` | 主加密密钥（凭据/LLM 密钥加密用），`openssl rand -hex 32`——**丢了加密数据全废，务必另存一份** |

按网络环境选填（详见 `.env.example` 内注释）：国内机 `CARGO_MIRROR` 默认 rsproxy
可用；海外机留空直连；宿主跑 Clash TUN 时给 `HTTPS_PROXY` 指到宿主代理端口。

```bash
docker compose up -d --build     # 首次构建约 10-20 分钟（分层缓存，之后升级只重编变更层）
curl http://localhost:8080/ready # {"status":"ready","migration_version":63} 即就绪（迁移自动跑）
```

浏览器开 `http://<主机>:8080`，用 `ADMIN_PASSWORD` 登录，第一步去「设置」配 LLM
供应商（蒸馏/嵌入要用），再建 API Key（`amk_`）给 AI 接 MCP：

```bash
claude mcp add --transport http engram http://<主机>:8080/mcp \
  --header "Authorization: Bearer amk_你的密钥"
```

## 反向代理与 HTTPS（公网部署必做）

Caddy 示例（自动 HTTPS）：

```text
engram.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

两个必配点：

- compose 里设 `AGENT_MEMORY_BIND=127.0.0.1`（app 只听本机回环，不裸暴露公网）
- `.env` 设 `AGENT_MEMORY_MCP_ALLOWED_HOSTS=engram.example.com`
  （MCP SDK 默认只放行 loopback Host，反代后不放行会 403）

## 升级

```bash
git pull && docker compose up -d --build
```

数据迁移在启动时自动执行（`/ready` 的 `migration_version` 会前进），
先起 db 后起 app 由 compose 编排，停机窗口 ≈ 重启时间。

## 数据

- 全部数据 bind mount 在宿主 `~/.engram/`（`postgres/`、`app/`）——
  **备份这一个目录即可**（建议停机或 `pg_dump` 一致性备份）
- 从已有本机实例迁数据：本机跑 `deploy/migrate-from-local.sh`；
  或手动走迁移 API（见仓库根 README 的「数据迁移」段）
- 凭据（credentials 域）与 LLM 供应商密钥**不随迁移包走**——
  加密绑定 `AGENT_MEMORY_MASTER_KEY`，目标机要自己重配

## 排障速查

| 症状 | 处置 |
|:--|:--|
| 构建时 buildkit 被 OOM 杀 | `.env` 设 `CARGO_JOBS=2`，宿主加 swap |
| crates.io 拉不动 / 403 | 换镜像源：`CARGO_MIRROR=sparse+https://mirrors.ustc.edu.cn/crates.io-index/`（海外机留空直连） |
| 容器内 URL 摄取全失败、SSRF 报私网地址 | 宿主跑 Clash TUN（fake-ip 198.18/15）污染 DNS——`.env` 设 `HTTPS_PROXY=http://host.docker.internal:<宿主代理端口>` |
| 反代后 MCP 403 | 漏配 `AGENT_MEMORY_MCP_ALLOWED_HOSTS` |
| 起不来要看日志 | `docker compose logs -f app`（或 `db`） |
