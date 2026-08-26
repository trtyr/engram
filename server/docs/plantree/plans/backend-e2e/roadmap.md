# Roadmap

## 状态：全部完成（2026-08-26）

`E2E_LLM_API_KEY` 注入下 `run_all.py` **13/13 PASS**（约 140 项断言，真 LLM 全链）；
无凭证时 LLM 依赖项明确 SKIP（exit 0），其余 PASS。

| ID | 任务 | 结果 |
|---|---|---|
| E0 | 公共编排库 `_lib/`（env/client/check）+ run_all.py | ✅ |
| E1 | 冒烟批：health_auth / apikeys_scopes / llm_provider_probe | ✅ 12+12+5 断言 |
| E2 | 核心域批：memory_distill / memory_read / knowledge_upload | ✅ 14+16+11 断言 |
| E3 | wiki 批：wiki_ingest / wiki_governance | ✅ 14+23 断言 |
| E4 | 跨域与系统批：unified_search / jobs / llm_settings / search_recall | ✅ 7+12+6+3 断言 |
| E5 | codegraph（本机 node+CLI 真跑） | ✅ 8 断言 |

## 关键实测发现（测试过程中抓到并处理）

1. **上游 dimensions 拒绝**：硅基流动 bge-m3 拒 `dimensions` 参数 → provider 已加 4xx 去参重试（E1 批修复）。
2. **LLM JSON 抖动**：wiki_generate 长 JSON 偶发语法错（MiniMax-M3）→ 测试侧换文本重试兜住（sha 幂等使同文本无法重入队，故换文本=换 sha）。
3. **蒸馏链是条件链**：arbitrate 全判重时不再入队 organize/persona——等待器按「必选/可选」语义实现。
4. **记账路径差异**：knowledge 管道 embed 直连不记账；`/test` 探测在 chat+embed 双 Ok 时记 purpose=test。

## 运行方式

```bash
export E2E_LLM_API_KEY=sk-xxx          # 可选；无则 LLM 依赖项 SKIP
cd scripts/e2e && python3 run_all.py   # 全量；单跑 python3 test_xxx.py
E2E_KEEP=1 python3 test_xxx.py         # 保留现场排障
```


## 批准口径

每批完成 = 脚本独立跑通（exit 0）+ `run_all.py` 全绿 + 关键失败场景（401/403/404）真的断言到。
