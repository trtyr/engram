//! 抽取效果 demo（P015 语义调整模型 v9）：原始会话 → 原子候选，真模型直跑看效果。
//! P015 落库切片：抽取 → 过滤 → 向量化 → INSERT 本地 demo_atoms 表 → 回读展示。
//!
//! 用法：
//!   DEMO_BASE_URL=https://newapi.trtyr.top/v1 \
//!   DEMO_API_KEY=<key> \
//!   DEMO_MODEL=MiniMax-M3 \
//!   DEMO_DATABASE_URL=postgresql://127.0.0.1/engram_ingest_demo \
//!   cargo run -p engram-distill --example extract_demo
//!
//! 入库目标 = 本地 demo_atoms 精简表（非正式 atoms——管道验证用，正式链路改造后续切片）。

use engram_llm::provider::LlmProvider as _;
use engram_llm::provider::OpenAiCompatProvider;
use engram_llm::types::{ChatMessage, ChatRequest, EmbedRequest};

#[tokio::main]
async fn main() {
    let base_url =
        std::env::var("DEMO_BASE_URL").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into());
    let api_key = std::env::var("DEMO_API_KEY").expect("DEMO_API_KEY 未设置");
    let model = std::env::var("DEMO_MODEL").unwrap_or_else(|_| "qwen/qwen3.8-27b:free".into());
    let db_url = std::env::var("DEMO_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://127.0.0.1/engram_ingest_demo".into());
    let embed_model =
        std::env::var("DEMO_EMBED_MODEL").unwrap_or_else(|_| "Qwen/Qwen3-Embedding-8B".into());

    let provider = OpenAiCompatProvider::new("demo", base_url, api_key);
    let system = engram_distill::prompts::extract_system();

    let samples: Vec<(&str, String)> = vec![
        (
            "样本1：多轮混合（偏好+人脉+项目事实陷阱+瞬态读数陷阱）",
            r#"【第1轮】用户：这个 bug 折腾我一下午了，端口老是占用。
【第2轮】AI：查了一下是 5432 被 postgres 占了，要我先帮你杀掉吗？
【第3轮】用户：杀吧。对了记住，我最烦别人催我 review，我自己会安排节奏，这话你以后提醒我也别提。
【第4轮】AI：好，已记住：不催 review。
【第5轮】用户：我们 engram 项目的 API 鉴权用的是 X-ENGRAM-TOKEN 这个头，你写文档的时候记得带。
【第6轮】AI：磁盘还剩 37G，临时文件在 /tmp/build-cache。
【第7轮】用户：嗯知道了。对了下个月我老婆生日，帮我到时候提醒一下，她叫林晚。"#
                .into(),
        ),
        (
            "样本2：单轮记忆型陈述（生产真实样本复刻）",
            r#"【第1轮】用户：review 未完成时不要 push。评审进行中（Draft/changes_requested）期间，修复在本地准备就行，等评审者确认后再一次 push。原因：多次小 push 让 MR head 不断漂移，评审者复评时会误判。这个适用于所有团队协作仓库。"#
                .into(),
        ),
        (
            "样本3：纯闲聊（应判不值得记）",
            r#"【第1轮】用户：今天天气真不错啊
【第2轮】AI：是啊，适合出去走走
【第3轮】用户：哈哈，中午吃了个外卖，还行
【第4轮】AI：吃饱了就行～"#.into(),
        ),
        (
            "样本4：瞬态读数 + 真实偏好混合",
            r#"【第1轮】用户：我笔记本电池循环次数到 340 次了，健康度 89%。
【第2轮】AI：还行，再用两年没问题。
【第3轮】用户：我买数码产品从来只买首发，等不了降价，这个习惯改不了。
【第4轮】AI：首发党哈哈。"#
                .into(),
        ),
    ];

    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[落库跳过] 本地库不可达（{e}）——只展示抽取效果");
            return;
        }
    };

    // demo 精简表：抽取输出直接落库 + 向量化（正式 atoms 管道改造后续切片）
    sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
        .execute(&pool)
        .await
        .expect("pgvector 扩展不可用");
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS demo_atoms (
             id UUID PRIMARY KEY,
             kind TEXT NOT NULL,
             content TEXT NOT NULL,
             confidence REAL NOT NULL,
             status TEXT NOT NULL DEFAULT 'active',
             embedding vector(1024),
             tsv tsvector,
             source_refs JSONB NOT NULL DEFAULT '[]',
             created_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .execute(&pool)
    .await
    .expect("建表失败");

    let mut inserted: Vec<(uuid::Uuid, String)> = Vec::new();

    for (title, dialogue) in &samples {
        println!("\n{}", "=".repeat(64));
        println!("【{}】", title);
        println!("{}", "=".repeat(64));
        let req = ChatRequest {
            model: model.clone(),
            messages: vec![
                ChatMessage::system(system.clone()),
                ChatMessage::user(dialogue.clone()),
            ],
            temperature: Some(0.1),
            json_mode: true,
            max_tokens: None,
            tools: None,
            // 关闭思考链（各家字段不同）：MiniMax-M3 用 thinking.type=disabled（官方文档）；
            // OpenRouter 统一 reasoning.enabled=false；DashScope qwen3 用 enable_thinking=false
            extras: Some({
                let mut m = serde_json::Map::new();
                let is_minimax = model.starts_with("MiniMax");
                if is_minimax {
                    m.insert("thinking".into(), serde_json::json!({ "type": "disabled" }));
                } else {
                    m.insert("reasoning".into(), serde_json::json!({ "enabled": false }));
                    m.insert("enable_thinking".into(), serde_json::json!(false));
                }
                m
            }),
        };
        match provider.chat(req).await {
            Ok(resp) => {
                println!(
                    "模型输出（{}ms / in:{} out:{} tok）：",
                    resp.latency_ms, resp.input_tokens, resp.output_tokens
                );
                match serde_json::from_str::<serde_json::Value>(resp.content.trim()) {
                    Ok(v) => {
                        let worth = v.get("worth_memorizing").and_then(|x| x.as_bool());
                        let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("-");
                        println!(
                            "  准入判断：worth_memorizing={:?}  reason：{}",
                            worth, reason
                        );
                        if let Some(atoms) = v.get("atoms").and_then(|x| x.as_array()) {
                            if atoms.is_empty() {
                                println!("  原子：（空）");
                            }
                            for (i, a) in atoms.iter().enumerate() {
                                let kind = a.get("kind").and_then(|x| x.as_str()).unwrap_or("?");
                                let conf =
                                    a.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0);
                                let strength =
                                    a.get("strength").and_then(|x| x.as_str()).unwrap_or("?");
                                let content =
                                    a.get("content").and_then(|x| x.as_str()).unwrap_or("?");
                                println!(
                                    "  [{}] ({} / {} / {:.2}) {}",
                                    i + 1,
                                    kind,
                                    strength,
                                    conf,
                                    content
                                );
                            }
                        }
                        if let Some(rels) = v.get("relations").and_then(|x| x.as_array()) {
                            for r in rels {
                                println!(
                                    "  关系：{} --{}--> {}",
                                    r.get("from").and_then(|x| x.as_str()).unwrap_or("?"),
                                    r.get("rel_type").and_then(|x| x.as_str()).unwrap_or("?"),
                                    r.get("to").and_then(|x| x.as_str()).unwrap_or("?"),
                                );
                            }
                        }

                        // ---- 落库：过滤 → 批量向量化 → INSERT ----
                        let mut kept: Vec<(uuid::Uuid, String, String, f64)> = Vec::new();
                        for a in v.get("atoms").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
                            let conf = a.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0);
                            let content = a.get("content").and_then(|x| x.as_str()).unwrap_or("");
                            let kind = a.get("kind").and_then(|x| x.as_str()).unwrap_or("fact");
                            if conf < 0.55 || content.is_empty() {
                                println!("  [丢弃] conf={conf:.2} {content}");
                                continue;
                            }
                            kept.push((uuid::Uuid::now_v7(), kind.into(), content.into(), conf));
                        }
                        if kept.is_empty() {
                            continue;
                        }
                        let inputs: Vec<String> =
                            kept.iter().map(|(_, _, c, _)| c.clone()).collect();
                        match provider
                            .embed(EmbedRequest {
                                model: embed_model.clone(),
                                inputs: inputs.clone(),
                                dimensions: Some(1024),
                            })
                            .await
                        {
                            Ok(emb) => {
                                println!(
                                    "  向量化：{} 条 → {} 维（{}ms）",
                                    inputs.len(),
                                    emb.embeddings.first().map(|e| e.len()).unwrap_or(0),
                                    emb.latency_ms
                                );
                                for (i, (id, kind, content, conf)) in kept.iter().enumerate() {
                                    let vec_text = format!(
                                        "[{}]",
                                        emb.embeddings[i]
                                            .iter()
                                            .map(|f| f.to_string())
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    );
                                    let r = sqlx::query(
                                        "INSERT INTO demo_atoms (id, kind, content, confidence, embedding, tsv, source_refs) \
                                         VALUES ($1, $2, $3, $4, $5::vector, to_tsvector('simple', $3), '[\"demo\"]'::jsonb)",
                                    )
                                    .bind(id)
                                    .bind(kind)
                                    .bind(content)
                                    .bind(*conf as f32)
                                    .bind(&vec_text)
                                    .execute(&pool)
                                    .await;
                                    match r {
                                        Ok(_) => println!(
                                            "  已落库 {} ({})",
                                            content,
                                            &id.to_string()[..8]
                                        ),
                                        Err(e) => println!("  [落库失败] {content}：{e}"),
                                    }
                                }
                            }
                            Err(e) => println!("  [向量化失败，本批未落库：{}]", e),
                        }
                        let _ = &mut inserted;
                    }
                    Err(e) => {
                        println!("  [JSON 解析失败：{}] 原文：{}", e, resp.content.trim());
                    }
                }
            }
            Err(e) => println!("  [调用失败：{}]", e),
        }
    }

    // ---- 落库回读展示 ----
    println!("\n{}", "=".repeat(64));
    println!("【回读 demo_atoms 全表】");
    println!("{}", "=".repeat(64));
    let rows = sqlx::query_as::<_, (uuid::Uuid, String, String, f32, Option<String>, Option<i32>)>(
        "SELECT id, kind, content, confidence, embedding::text, array_length(embedding::text::float4[], 1) FROM demo_atoms ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    for (id, kind, content, conf, _emb, dims) in rows {
        println!(
            "  {} {:9} {:.2}  {}\n         id={} 向量维度={:?}",
            &id.to_string()[..8],
            kind,
            conf,
            content,
            &id.to_string()[..8],
            dims,
        );
    }
    let _ = inserted;
}
