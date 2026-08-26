# Open Questions（需要用户提供的输入）

## 必需

1. **LLM provider 凭证**（已确认可用，2026-08-26 实测）：
   - `base_url`: `https://newapi.trtyr.top`（用户 newapi 网关）
   - `api_key`: 环境变量 `E2E_LLM_API_KEY` 注入（用户已提供，脚本/文档不落盘）
   - chat 模型：`MiniMax-M3`（JSON 模式实测 OK）
   - embedding 模型：`BAAI/bge-m3`（默认输出 1024 维，**不支持 `dimensions` 参数**——
     上游硅基流动会报 bad_response_status_code；本项目的 EmbedRequest 传了
     `dimensions: Some(1024)`，**需要后端适配**：模型不支持 dimensions 时省略该参数
     改靠默认维度，E0 里先验证）

## 已确认（无需再问）

2. **PG**：本机 Homebrew PostgreSQL 16（127.0.0.1:5432，pgvector 0.8.6 已装）——E2E 建独立库 `agent_memory_e2e`，不碰用户库、不用 Docker。
3. **服务**：脚本自动 `cargo build` 并拉起二进制（连本机 PG 的 E2E 库，随机端口）。
5. **Python**：按 3.10+ 写（`X | None` 语法），仅依赖 `requests`。

## 待定

6. URL 摄取（原 #6 计划项已并入 #5 可选段）：E2E 机器要出外网抓真实页面，网络受限时跳过——默认只测上传通道。
