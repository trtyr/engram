//! 抽取效果 demo（P015 语义调整模型 v9）：原始会话 → 原子候选，真模型直跑看效果。
//!
//! 用法：
//!   DEMO_BASE_URL=https://openrouter.ai/api/v1 \
//!   DEMO_API_KEY=<key> \
//!   DEMO_MODEL=qwen/qwen3.8-27b:free \
//!   cargo run -p engram-distill --example extract_demo
//!
//! 不入库不入链——纯 prompt 效果验证。

use engram_llm::provider::OpenAiCompatProvider;
use engram_llm::types::{ChatMessage, ChatRequest};
use engram_llm::provider::LlmProvider as _;

#[tokio::main]
async fn main() {
    let base_url = std::env::var("DEMO_BASE_URL").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into());
    let api_key = std::env::var("DEMO_API_KEY").expect("DEMO_API_KEY 未设置");
    let model = std::env::var("DEMO_MODEL").unwrap_or_else(|_| "qwen/qwen3.8-27b:free".into());

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

    for (title, dialogue) in &samples {
        println!("\n{}", "=".repeat(64));
        println!("【{}】", title);
        println!("{}", "=".repeat(64));
        let req = ChatRequest {
            model: model.clone(),
            messages: vec![ChatMessage::system(system.clone()), ChatMessage::user(dialogue.clone())],
            temperature: Some(0.1),
            json_mode: true,
            max_tokens: None,
            tools: None,
            // 关闭思考链：OpenRouter 统一 reasoning 对象；qwen3 系另认 enable_thinking
            extras: Some({
                let mut m = serde_json::Map::new();
                m.insert("reasoning".into(), serde_json::json!({ "enabled": false }));
                m.insert("enable_thinking".into(), serde_json::json!(false));
                m
            }),
        };
        match provider.chat(req).await {
            Ok(resp) => {
                println!("模型输出（{}ms / in:{} out:{} tok）：", resp.latency_ms, resp.input_tokens, resp.output_tokens);
                match serde_json::from_str::<serde_json::Value>(resp.content.trim()) {
                    Ok(v) => {
                        let worth = v.get("worth_memorizing").and_then(|x| x.as_bool());
                        let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("-");
                        println!("  准入判断：worth_memorizing={:?}  reason：{}", worth, reason);
                        if let Some(atoms) = v.get("atoms").and_then(|x| x.as_array()) {
                            if atoms.is_empty() {
                                println!("  原子：（空）");
                            }
                            for (i, a) in atoms.iter().enumerate() {
                                let kind = a.get("kind").and_then(|x| x.as_str()).unwrap_or("?");
                                let conf = a.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0);
                                let strength = a.get("strength").and_then(|x| x.as_str()).unwrap_or("?");
                                let content = a.get("content").and_then(|x| x.as_str()).unwrap_or("?");
                                println!("  [{}] ({} / {} / {:.2}) {}", i + 1, kind, strength, conf, content);
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
                    }
                    Err(e) => {
                        println!("  [JSON 解析失败：{}] 原文：{}", e, resp.content.trim());
                    }
                }
            }
            Err(e) => println!("  [调用失败：{}]", e),
        }
    }
}
