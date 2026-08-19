//! 领域层：memory / knowledge / wiki / codegraph 各域服务与跨域编排。
//!
//! 依赖方向（见 docs/plantree/baseline/module-map.md）：
//! `api → core → (storage, llm, jobs, search, distill, wiki-engine, cg-bridge)`
//! core 不被 storage/llm/jobs 等基础 crate 依赖。
