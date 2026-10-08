//! P018-Q006：ATOM_MAX_CHARS 双写同步测试。
//!
//! distill 不依赖 core，`extract_model::ATOM_MAX_CHARS` 与
//! `core::memory::ATOM_MAX_CHARS` 各定义一份（配对常量）。api 同时依赖两者，
//! 在此强制相等——漂移即红，替代原「注释人工同步」。

#[test]
fn atom_max_chars_sync() {
    assert_eq!(
        engram_core::memory::ATOM_MAX_CHARS,
        engram_distill::extract_model::ATOM_MAX_CHARS,
        "ATOM_MAX_CHARS 双写漂移——两处需同步修改（core/memory.rs 与 distill/extract_model.rs）"
    );
}
