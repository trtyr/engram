# Roadmap

## In Progress

| ID | 任务 | 产物 |
|---|---|---|
| E0 | 公共编排库 `_lib/`（env/client/check）+ run_all.py | ✅ Done |
| E1 | 冒烟批：health_auth + apikeys_scopes + llm_provider_probe | ✅ Done（3 脚本 20+ 断言全绿） |
| E2 | 核心域批：test_memory_distill + test_memory_read + test_knowledge_upload | #3 #4 #5 |
| E3 | wiki 批：test_wiki_ingest + test_wiki_governance | #6 #7 |
| E4 | 跨域与系统批：test_unified_search + test_jobs + test_llm_settings + test_search_recall | #8~#11 |
| E5 | 可选批：test_codegraph（环境允许时） | #12 |

## Next

- E2 核心域批（真 LLM 蒸馏链 + 知识摄取，跑起来每脚本约 1~3 分钟）。

## Done

- **E0+E1（2026-08-26）**：`scripts/e2e/` 基建落地（零 Docker：本机 PG 独立库 + cargo build + 随机端口拉服务）；
  冒烟 3 脚本全绿。附带成果：**发现并修复 embed dimensions 兼容问题**（硅基流动上游拒绝
  `dimensions` 参数 → provider 4xx 时去掉重试；真网关实测 `embed 414ms×1024维` 通过），
  llm crate 12 tests passed。

## 批准口径

每批完成 = 脚本独立跑通（exit 0）+ `run_all.py` 全绿 + 关键失败场景（401/403/404）真的断言到。
